use super::prompt_cache_and_timeseries_shared as prompt_shared;
use super::*;
use anyhow::anyhow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};
use tokio::{
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, warn};

#[cfg(test)]
#[derive(sqlx::FromRow)]
struct TimeseriesMinuteProjectionRow {
    minute_start_epoch: i64,
    records_json: String,
    max_row_id: i64,
}

#[derive(sqlx::FromRow)]
struct TimeseriesMinuteProjectionV2Row {
    minute_start_epoch: i64,
    aggregate_json: String,
    total_latency_samples_json: String,
    first_byte_samples_json: String,
    first_response_byte_total_samples_json: String,
    first_token_samples_json: String,
    max_row_id: i64,
    coverage_state: String,
}

fn fold_minute_projection_aggregates(
    minute_aggregates: BTreeMap<i64, BucketAggregate>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<BTreeMap<i64, BucketAggregate>, ApiError> {
    let mut buckets = BTreeMap::new();
    for (minute_epoch, aggregate) in minute_aggregates {
        let bucket_epoch =
            align_reporting_bucket_epoch(minute_epoch, bucket_seconds, reporting_tz)?;
        merge_timeseries_bucket_aggregate(buckets.entry(bucket_epoch).or_default(), aggregate);
    }
    Ok(buckets)
}

fn timeseries_projection_scope(source_scope: InvocationSourceScope) -> &'static str {
    match source_scope {
        InvocationSourceScope::All => "all",
        InvocationSourceScope::ProxyOnly => "proxy_only",
    }
}

fn complete_minute_bounds(start: DateTime<Utc>, end: DateTime<Utc>) -> (i64, i64) {
    let start_epoch = (start.timestamp() + 59).div_euclid(60) * 60;
    let end_epoch = end.timestamp().div_euclid(60) * 60;
    (start_epoch, end_epoch.max(start_epoch))
}

#[cfg(test)]
async fn load_timeseries_minute_projection_records(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<Option<(Vec<InvocationAggregateRecord>, i64)>, ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    let expected = (minute_end - minute_start).div_euclid(60);
    if expected <= 0 {
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, TimeseriesMinuteProjectionRow>(
        "SELECT minute_start_epoch, records_json, max_row_id FROM timeseries_minute_projection_records WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2 AND source_scope = ?3 AND upstream_account_key = ?4 ORDER BY minute_start_epoch",
    )
    .bind(minute_start)
    .bind(minute_end)
    .bind(timeseries_projection_scope(source_scope))
    .bind(upstream_account_id.unwrap_or(-1))
    .fetch_all(pool)
    .await?;
    if rows.len() as i64 != expected {
        return Ok(None);
    }
    let mut records = Vec::new();
    let mut cursor = 0;
    for row in rows {
        cursor = cursor.max(row.max_row_id);
        records.extend(
            serde_json::from_str::<Vec<InvocationAggregateRecord>>(&row.records_json).map_err(
                |err| ApiError::from(anyhow!("invalid minute projection record: {err}")),
            )?,
        );
    }
    Ok(Some((records, cursor)))
}

#[cfg(test)]
async fn store_timeseries_minute_projection_records(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    records: &[InvocationAggregateRecord],
) -> Result<(), ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    if minute_end <= minute_start {
        return Ok(());
    }
    let mut by_minute: HashMap<i64, Vec<InvocationAggregateRecord>> = HashMap::new();
    for record in records {
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let minute = occurred.timestamp().div_euclid(60) * 60;
        if minute >= minute_start && minute < minute_end {
            by_minute.entry(minute).or_default().push(record.clone());
        }
    }
    let mut tx = pool.begin().await?;
    let mut projected_rows = Vec::new();
    for minute in (minute_start..minute_end).step_by(60_usize) {
        let minute_records = by_minute.remove(&minute).unwrap_or_default();
        let max_row_id = minute_records
            .iter()
            .map(|record| record.id)
            .max()
            .unwrap_or(0);
        projected_rows.push((
            minute,
            serde_json::to_string(&minute_records).map_err(ApiError::from)?,
            max_row_id,
        ));
    }
    for chunk in projected_rows.chunks(128) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO timeseries_minute_projection_records (minute_start_epoch, source_scope, upstream_account_key, records_json, max_row_id) ",
        );
        query.push_values(chunk, |mut values, (minute, records_json, max_row_id)| {
            values
                .push_bind(*minute)
                .push_bind(timeseries_projection_scope(source_scope))
                .push_bind(upstream_account_id.unwrap_or(-1))
                .push_bind(records_json)
                .push_bind(*max_row_id);
        });
        query.push(
            " ON CONFLICT(minute_start_epoch, source_scope, upstream_account_key) DO UPDATE SET records_json = excluded.records_json, max_row_id = excluded.max_row_id",
        );
        query.build().execute(tx.as_mut()).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn load_timeseries_minute_projection_v2(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<Option<(BTreeMap<i64, BucketAggregate>, i64)>, ApiError> {
    let projection_ready = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_error FROM timeseries_minute_projection_v2_state WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_optional(pool)
    .await?
    .flatten()
    .is_some_and(|state| state == "ready");
    if !projection_ready {
        return Ok(None);
    }
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    let expected = (minute_end - minute_start).div_euclid(60);
    if expected <= 0 {
        return Ok(None);
    }
    let rows = sqlx::query_as::<_, TimeseriesMinuteProjectionV2Row>(
        "SELECT minute_start_epoch, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2 AND source_scope = ?3 AND upstream_account_key = ?4 AND coverage_state = 'ready' ORDER BY minute_start_epoch",
    )
    .bind(minute_start)
    .bind(minute_end)
    .bind(timeseries_projection_scope(source_scope))
    .bind(upstream_account_id.unwrap_or(-1))
    .fetch_all(pool)
    .await?;
    if rows.len() as i64 != expected {
        return Ok(None);
    }
    let mut aggregates = BTreeMap::new();
    let mut cursor = 0;
    for row in rows {
        debug_assert_eq!(row.coverage_state, "ready");
        let mut aggregate = serde_json::from_str::<BucketAggregate>(&row.aggregate_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 minute aggregate: {err}")))?;
        // Keep the sample payloads independently addressable so a corrupt aggregate payload
        // cannot silently turn an exact P95 into a histogram approximation.
        aggregate.first_byte_ttfb_values = serde_json::from_str(&row.first_byte_samples_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 first-byte samples: {err}")))?;
        aggregate.first_response_byte_total_values =
            serde_json::from_str(&row.first_response_byte_total_samples_json).map_err(|err| {
                ApiError::from(anyhow!("invalid v2 first-response samples: {err}"))
            })?;
        aggregate.first_token_values = serde_json::from_str(&row.first_token_samples_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 first-token samples: {err}")))?;
        cursor = cursor.max(row.max_row_id);
        aggregates.insert(row.minute_start_epoch, aggregate);
    }
    Ok(Some((aggregates, cursor)))
}

async fn store_timeseries_minute_projection_v2(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    records: &[InvocationAggregateRecord],
) -> Result<(), ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    if minute_end <= minute_start {
        return Ok(());
    }
    let mut by_minute: HashMap<i64, BucketAggregate> = HashMap::new();
    let mut max_row_ids = HashMap::new();
    for record in records {
        if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let minute = occurred.timestamp().div_euclid(60) * 60;
        if minute < minute_start || minute >= minute_end {
            continue;
        }
        add_exact_record_to_timeseries_aggregate(by_minute.entry(minute).or_default(), record);
        max_row_ids
            .entry(minute)
            .and_modify(|max_row_id: &mut i64| *max_row_id = (*max_row_id).max(record.id))
            .or_insert(record.id);
    }
    let mut tx = pool.begin().await?;
    for minute in (minute_start..minute_end).step_by(60_usize) {
        let aggregate = by_minute.remove(&minute).unwrap_or_default();
        let aggregate_json = serde_json::to_string(&aggregate).map_err(ApiError::from)?;
        let total_latency_samples_json = "[]";
        let first_byte_samples_json =
            serde_json::to_string(&aggregate.first_byte_ttfb_values).map_err(ApiError::from)?;
        let first_response_byte_total_samples_json =
            serde_json::to_string(&aggregate.first_response_byte_total_values)
                .map_err(ApiError::from)?;
        let first_token_samples_json =
            serde_json::to_string(&aggregate.first_token_values).map_err(ApiError::from)?;
        sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', datetime('now')) ON CONFLICT(minute_start_epoch, source_scope, upstream_account_key) DO UPDATE SET aggregate_json = excluded.aggregate_json, total_latency_samples_json = excluded.total_latency_samples_json, first_byte_samples_json = excluded.first_byte_samples_json, first_response_byte_total_samples_json = excluded.first_response_byte_total_samples_json, first_token_samples_json = excluded.first_token_samples_json, max_row_id = excluded.max_row_id, coverage_state = 'ready', updated_at = excluded.updated_at WHERE excluded.max_row_id > timeseries_minute_projection_v2.max_row_id OR (timeseries_minute_projection_v2.coverage_state = 'warming' AND excluded.max_row_id = timeseries_minute_projection_v2.max_row_id)",
        )
        .bind(minute)
        .bind(timeseries_projection_scope(source_scope))
        .bind(upstream_account_id.unwrap_or(-1))
        .bind(aggregate_json)
        .bind(total_latency_samples_json)
        .bind(first_byte_samples_json)
        .bind(first_response_byte_total_samples_json)
        .bind(first_token_samples_json)
        .bind(max_row_ids.remove(&minute).unwrap_or(0))
        .execute(tx.as_mut())
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub(crate) fn add_timeseries_terminal_delta_to_aggregate(
    entry: &mut BucketAggregate,
    delta: &TimeseriesTerminalDelta,
) {
    entry.total_count += 1;
    let classification = resolve_failure_classification(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    );
    let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    if is_success_like {
        entry.success_count += 1;
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        delta.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        entry.failure_count += 1;
    }
    let latency_status = is_success_like
        .then_some("success")
        .or(delta.status.as_deref());
    entry.record_total_latency_sample(delta.t_total_ms);
    entry.record_exact_ttfb_sample(latency_status, delta.t_upstream_ttfb_ms);
    entry.record_exact_first_response_byte_total_sample(
        delta.t_req_read_ms,
        delta.t_req_parse_ms,
        delta.t_upstream_connect_ms,
        delta.t_upstream_ttfb_ms,
    );
    entry.record_first_token_sample(delta.first_token_ms);
    entry.total_tokens += delta.total_tokens.unwrap_or_default();
    entry.cache_input_tokens += delta.cache_input_tokens.unwrap_or_default();
    let cost = delta.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        delta.status.as_deref(),
        delta.error_message.as_deref(),
        delta.failure_kind.as_deref(),
        delta.failure_class.as_deref(),
        delta.is_actionable.map(i64::from),
    ) {
        entry.non_success_cost += cost;
    }
}

fn add_pending_timeseries_deltas(
    state: &AppState,
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    max_row_id: Option<i64>,
    snapshot_id: i64,
) -> Result<usize, ApiError> {
    let mut applied = 0;
    let selection = TimeseriesProjectionSelection {
        source_scope: timeseries_projection_scope(source_scope),
        upstream_account_id,
    };
    for (_, row_id, delta) in state
        .terminal_projection_hub
        .pending_timeseries_deltas_for_selection(selection, 10_000)
    {
        if max_row_id.is_some_and(|cursor| row_id > cursor) {
            // Rows above the projection cursor are already represented by the SQL tail.
            continue;
        }
        if row_id > snapshot_id {
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && delta.source != SOURCE_PROXY {
            continue;
        }
        if upstream_account_id
            .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
        {
            continue;
        }
        let Some(occurred) = parse_to_utc_datetime(&delta.occurred_at) else {
            continue;
        };
        if occurred < start || occurred >= end {
            continue;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
        add_timeseries_terminal_delta_to_aggregate(
            aggregates.entry(bucket_epoch).or_default(),
            &delta,
        );
        applied += 1;
    }
    Ok(applied)
}

fn timeseries_projection_snapshot_record(
    record: &InvocationAggregateRecord,
) -> TimeseriesProjectionSnapshotRecord {
    TimeseriesProjectionSnapshotRecord {
        row_id: record.id,
        invoke_id: record.invoke_id.clone(),
        occurred_at: record.occurred_at.clone(),
        delta: TimeseriesTerminalDelta {
            occurred_at: record.occurred_at.clone(),
            source: String::new(),
            upstream_account_id: None,
            status: record.status.clone(),
            error_message: record.error_message.clone(),
            failure_kind: record.failure_kind.clone(),
            failure_class: record.failure_class.clone(),
            is_actionable: record.is_actionable.map(|value| value != 0),
            total_tokens: record.total_tokens,
            cache_input_tokens: record.cache_input_tokens,
            cost: record.cost,
            t_total_ms: record.t_total_ms,
            t_req_read_ms: record.t_req_read_ms,
            t_req_parse_ms: record.t_req_parse_ms,
            t_upstream_connect_ms: record.t_upstream_connect_ms,
            t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
            first_token_ms: record.first_token_ms,
        },
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct TimeseriesMinuteProjectionKey {
    minute_start_epoch: i64,
    source_scope: &'static str,
    upstream_account_key: i64,
}

fn add_timeseries_delta_projection_keys(
    grouped: &mut HashMap<TimeseriesMinuteProjectionKey, Vec<(i64, TimeseriesTerminalDelta)>>,
    row_id: i64,
    delta: TimeseriesTerminalDelta,
) {
    let Some(occurred) = parse_to_utc_datetime(&delta.occurred_at) else {
        return;
    };
    let minute_start_epoch = occurred.timestamp().div_euclid(60) * 60;
    let account_key = delta.upstream_account_id.unwrap_or(-1);
    let mut keys = vec![TimeseriesMinuteProjectionKey {
        minute_start_epoch,
        source_scope: "all",
        upstream_account_key: -1,
    }];
    if account_key != -1 {
        keys.push(TimeseriesMinuteProjectionKey {
            minute_start_epoch,
            source_scope: "all",
            upstream_account_key: account_key,
        });
    }
    if delta.source == SOURCE_PROXY {
        keys.push(TimeseriesMinuteProjectionKey {
            minute_start_epoch,
            source_scope: "proxy_only",
            upstream_account_key: -1,
        });
        if account_key != -1 {
            keys.push(TimeseriesMinuteProjectionKey {
                minute_start_epoch,
                source_scope: "proxy_only",
                upstream_account_key: account_key,
            });
        }
    }
    for key in keys {
        grouped
            .entry(key)
            .or_default()
            .push((row_id, delta.clone()));
    }
}

fn merge_timeseries_bucket_aggregate(target: &mut BucketAggregate, source: BucketAggregate) {
    target.total_count += source.total_count;
    target.success_count += source.success_count;
    target.failure_count += source.failure_count;
    target.in_flight_count += source.in_flight_count;
    target.in_flight_phase_counts.queued += source.in_flight_phase_counts.queued;
    target.in_flight_phase_counts.requesting += source.in_flight_phase_counts.requesting;
    target.in_flight_phase_counts.responding += source.in_flight_phase_counts.responding;
    target.total_tokens += source.total_tokens;
    target.cache_input_tokens += source.cache_input_tokens;
    target.total_cost += source.total_cost;
    target.non_success_cost += source.non_success_cost;
    target.total_latency_sum_ms += source.total_latency_sum_ms;
    target.total_latency_sample_count += source.total_latency_sample_count;
    target.first_byte_ttfb_sum_ms += source.first_byte_ttfb_sum_ms;
    target.first_byte_sample_count += source.first_byte_sample_count;
    target.first_response_byte_total_sum_ms += source.first_response_byte_total_sum_ms;
    target.first_response_byte_total_sample_count += source.first_response_byte_total_sample_count;
    target.first_token_sum_ms += source.first_token_sum_ms;
    target.first_token_sample_count += source.first_token_sample_count;
    target
        .first_byte_ttfb_values
        .extend(source.first_byte_ttfb_values);
    target
        .first_response_byte_total_values
        .extend(source.first_response_byte_total_values);
    target.first_token_values.extend(source.first_token_values);
    let first_byte_histogram = source.first_byte_histogram;
    if target.first_byte_histogram.is_empty() {
        target.first_byte_histogram = first_byte_histogram;
    } else {
        for (target_value, source_value) in target
            .first_byte_histogram
            .iter_mut()
            .zip(first_byte_histogram)
        {
            *target_value += source_value;
        }
    }
    let first_response_byte_total_histogram = source.first_response_byte_total_histogram;
    if target.first_response_byte_total_histogram.is_empty() {
        target.first_response_byte_total_histogram = first_response_byte_total_histogram;
    } else {
        for (target_value, source_value) in target
            .first_response_byte_total_histogram
            .iter_mut()
            .zip(first_response_byte_total_histogram)
        {
            *target_value += source_value;
        }
    }
    let first_token_histogram = source.first_token_histogram;
    if target.first_token_histogram.is_empty() {
        target.first_token_histogram = first_token_histogram;
    } else {
        for (target_value, source_value) in target
            .first_token_histogram
            .iter_mut()
            .zip(first_token_histogram)
        {
            *target_value += source_value;
        }
    }
}

async fn load_timeseries_minute_projection_v2_key_tx(
    tx: &mut SqliteConnection,
    key: &TimeseriesMinuteProjectionKey,
) -> Result<Option<(BucketAggregate, i64)>, ApiError> {
    let row = sqlx::query_as::<_, TimeseriesMinuteProjectionV2Row>(
        "SELECT minute_start_epoch, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = ?2 AND upstream_account_key = ?3",
    )
    .bind(key.minute_start_epoch)
    .bind(key.source_scope)
    .bind(key.upstream_account_key)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.coverage_state != "ready" {
        return Ok(None);
    }
    let mut aggregate = serde_json::from_str::<BucketAggregate>(&row.aggregate_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 minute aggregate: {err}")))?;
    aggregate.first_byte_ttfb_values = serde_json::from_str(&row.first_byte_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 first-byte samples: {err}")))?;
    aggregate.first_response_byte_total_values =
        serde_json::from_str(&row.first_response_byte_total_samples_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 first-response samples: {err}")))?;
    aggregate.first_token_values = serde_json::from_str(&row.first_token_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 first-token samples: {err}")))?;
    Ok(Some((aggregate, row.max_row_id)))
}

async fn rebuild_timeseries_minute_projection_v2_key_tx(
    tx: &mut SqliteConnection,
    key: &TimeseriesMinuteProjectionKey,
) -> Result<(BucketAggregate, i64), ApiError> {
    let start = Utc
        .timestamp_opt(key.minute_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid minute projection start")))?;
    let end = start + ChronoDuration::minutes(1);
    let range = ExactUtcRange { start, end };
    let source_scope = if key.source_scope == "proxy_only" {
        InvocationSourceScope::ProxyOnly
    } else {
        InvocationSourceScope::All
    };
    let records = if key.upstream_account_key == -1 {
        query_invocation_aggregate_records_from_live_range_tx(tx, range, source_scope, None, None)
            .await?
    } else {
        query_invocation_aggregate_records_from_live_range_tx_for_account(
            tx,
            range,
            source_scope,
            None,
            None,
            key.upstream_account_key,
        )
        .await?
    };
    let mut aggregate = BucketAggregate::default();
    let mut max_row_id = 0;
    for record in records {
        if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            continue;
        }
        max_row_id = max_row_id.max(record.id);
        add_exact_record_to_timeseries_aggregate(&mut aggregate, &record);
    }
    Ok((aggregate, max_row_id))
}

async fn upsert_timeseries_minute_projection_v2_key_tx(
    tx: &mut SqliteConnection,
    key: &TimeseriesMinuteProjectionKey,
    aggregate: &BucketAggregate,
    max_row_id: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', datetime('now')) ON CONFLICT(minute_start_epoch, source_scope, upstream_account_key) DO UPDATE SET aggregate_json = excluded.aggregate_json, total_latency_samples_json = excluded.total_latency_samples_json, first_byte_samples_json = excluded.first_byte_samples_json, first_response_byte_total_samples_json = excluded.first_response_byte_total_samples_json, first_token_samples_json = excluded.first_token_samples_json, max_row_id = MAX(timeseries_minute_projection_v2.max_row_id, excluded.max_row_id), coverage_state = 'ready', updated_at = excluded.updated_at",
    )
    .bind(key.minute_start_epoch)
    .bind(key.source_scope)
    .bind(key.upstream_account_key)
    .bind(serde_json::to_string(aggregate).map_err(ApiError::from)?)
    .bind("[]")
    .bind(serde_json::to_string(&aggregate.first_byte_ttfb_values).map_err(ApiError::from)?)
    .bind(
        serde_json::to_string(&aggregate.first_response_byte_total_values)
            .map_err(ApiError::from)?,
    )
    .bind(serde_json::to_string(&aggregate.first_token_values).map_err(ApiError::from)?)
    .bind(max_row_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) async fn flush_timeseries_minute_projection(
    state: &AppState,
    trigger: &'static str,
) -> Result<(), ApiError> {
    let pending = state
        .terminal_projection_hub
        .pending_timeseries_deltas(10_000);
    let coverage_invalidation_generation = state
        .terminal_projection_hub
        .timeseries_coverage_invalidation_pending();
    if pending.is_empty() && coverage_invalidation_generation.is_none() {
        return Ok(());
    }
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _permit = match gate.try_begin_background("timeseries_minute_projection_flush") {
        Ok(permit) => permit,
        Err(reason) => {
            debug!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                trigger,
                gate_outcome = "deferred",
                defer_reason = "writer_pressure",
                %reason,
                "minute projection flush deferred by database pressure"
            );
            return Ok(());
        }
    };
    let started = Instant::now();
    let mut grouped = HashMap::new();
    let mut flushed_event_ids = Vec::with_capacity(pending.len());
    for (event_id, row_id, delta) in pending {
        flushed_event_ids.push(event_id);
        add_timeseries_delta_projection_keys(&mut grouped, row_id, delta);
    }
    let mut tx = state.pool.begin().await?;
    if coverage_invalidation_generation.is_some() {
        // A Hub admission rejection may have omitted a terminal update for an existing
        // row. Never keep any minute marked ready until exact fallback has rebuilt it.
        sqlx::query("UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming'")
            .execute(tx.as_mut())
            .await?;
    }
    let mut written_key_count = 0usize;
    let mut exact_fallback_minute_count = 0usize;
    for (key, mut deltas) in grouped {
        deltas.sort_by_key(|(row_id, _)| *row_id);
        let (aggregate, max_row_id) = if let Some((aggregate, existing_max_row_id)) =
            load_timeseries_minute_projection_v2_key_tx(tx.as_mut(), &key).await?
        {
            if deltas
                .iter()
                .any(|(row_id, _)| *row_id <= existing_max_row_id)
            {
                // A terminal record can update an older running row. Its row ID is not a
                // change cursor, so an incremental merge would silently omit that terminal.
                exact_fallback_minute_count += 1;
                rebuild_timeseries_minute_projection_v2_key_tx(tx.as_mut(), &key).await?
            } else {
                let mut aggregate = aggregate;
                let mut max_row_id = existing_max_row_id;
                for (row_id, delta) in deltas {
                    add_timeseries_terminal_delta_to_aggregate(&mut aggregate, &delta);
                    max_row_id = max_row_id.max(row_id);
                }
                (aggregate, max_row_id)
            }
        } else {
            // A delta alone cannot prove that a minute is complete: a process can start
            // mid-minute or recover after an earlier terminal write. Rebuild only this
            // minute before publishing it as ready, rather than storing a partial total.
            exact_fallback_minute_count += 1;
            rebuild_timeseries_minute_projection_v2_key_tx(tx.as_mut(), &key).await?
        };
        upsert_timeseries_minute_projection_v2_key_tx(tx.as_mut(), &key, &aggregate, max_row_id)
            .await?;
        written_key_count += 1;
    }
    tx.commit().await?;
    if let Some(generation) = coverage_invalidation_generation {
        state
            .terminal_projection_hub
            .complete_timeseries_coverage_invalidation(generation);
    }
    state
        .terminal_projection_hub
        .mark_timeseries_deltas_flushed(&flushed_event_ids);
    debug!(
        route = "timeseries_projection",
        builder = "minute_projection_v2",
        trigger,
        response_source = "memory_overlay_flush",
        event_count = flushed_event_ids.len(),
        minute_rollup_count = written_key_count,
        exact_fallback_minute_count,
        coverage_invalidation_pending = coverage_invalidation_generation.is_some(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "flushed terminal deltas into minute projection"
    );
    Ok(())
}

pub(crate) fn spawn_timeseries_minute_projection_supervisor(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = prepare_timeseries_minute_projection_after_restart(state.as_ref()).await
        {
            warn!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                ?error,
                "failed to invalidate minute projection during startup recovery"
            );
        }
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = ticker.tick() => {
                    if let Err(error) = flush_timeseries_minute_projection(state.as_ref(), "terminal_deadline").await {
                        warn!(
                            route = "timeseries_projection",
                            builder = "minute_projection_v2",
                            trigger = "terminal_deadline",
                            ?error,
                            "minute projection flush failed"
                        );
                    }
                }
            }
        }
    })
}

async fn prepare_timeseries_minute_projection_after_restart(
    state: &AppState,
) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2_state (consumer, cursor_row_id, last_flush_at, last_error, updated_at) VALUES ('timeseries_minute_v2', 0, NULL, 'warming', datetime('now')) ON CONFLICT(consumer) DO UPDATE SET last_error = 'warming', updated_at = excluded.updated_at",
    )
    .execute(tx.as_mut())
    .await?;
    sqlx::query("UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming'")
        .execute(tx.as_mut())
        .await?;
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_state SET last_error = 'ready', updated_at = datetime('now') WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    debug!(
        route = "timeseries_projection",
        builder = "minute_projection_v2",
        response_source = "startup_invalidation",
        "invalidated stale minute coverage before accepting projection reads"
    );
    Ok(())
}

#[cfg(test)]
mod minute_projection_tests {
    use super::*;

    fn record(id: i64, occurred_at: &str) -> InvocationAggregateRecord {
        InvocationAggregateRecord {
            id,
            invoke_id: format!("invoke-{id}"),
            occurred_at: occurred_at.to_string(),
            status: Some("success".to_string()),
            total_tokens: Some(3),
            cache_input_tokens: Some(1),
            cost: Some(0.25),
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: Some(0),
            live_phase: None,
            t_total_ms: Some(10.0),
            t_req_read_ms: Some(1.0),
            t_req_parse_ms: Some(1.0),
            t_upstream_connect_ms: Some(2.0),
            t_upstream_ttfb_ms: Some(3.0),
            first_token_ms: Some(4.0),
            t_upstream_stream_ms: Some(5.0),
            t_resp_parse_ms: Some(1.0),
            t_persist_ms: Some(1.0),
        }
    }

    #[tokio::test]
    async fn minute_projection_returns_exact_records_and_cursor_for_covered_minutes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_records (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, records_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create projection table");
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
        let end = Utc.with_ymd_and_hms(2026, 8, 1, 0, 3, 0).single().unwrap();
        store_timeseries_minute_projection_records(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &[record(17, "2026-08-01 08:01:12")],
        )
        .await
        .expect("store projection");

        let (records, cursor) = load_timeseries_minute_projection_records(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
        )
        .await
        .expect("load projection")
        .expect("complete minute coverage");
        assert_eq!(cursor, 17);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].invoke_id, "invoke-17");
    }

    #[tokio::test]
    async fn minute_projection_v2_preserves_exact_latency_samples_without_raw_records() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2 (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, aggregate_json TEXT NOT NULL, total_latency_samples_json TEXT NOT NULL, first_byte_samples_json TEXT NOT NULL, first_response_byte_total_samples_json TEXT NOT NULL, first_token_samples_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, coverage_state TEXT NOT NULL DEFAULT 'warming', updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection table");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection state table");
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, last_error) VALUES ('timeseries_minute_v2', 'ready')",
        )
        .execute(&pool)
        .await
        .expect("mark v2 projection ready");
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
        let end = Utc.with_ymd_and_hms(2026, 8, 1, 0, 2, 0).single().unwrap();
        let mut first = record(17, "2026-08-01 08:00:12");
        first.t_upstream_ttfb_ms = Some(10.0);
        first.first_token_ms = Some(11.0);
        let mut second = record(18, "2026-08-01 08:00:30");
        second.t_upstream_ttfb_ms = Some(100.0);
        second.first_token_ms = Some(101.0);
        let mut in_flight = record(19, "2026-08-01 08:00:40");
        in_flight.status = Some("running".to_string());
        store_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &[first, second, in_flight],
        )
        .await
        .expect("store v2 projection");

        let (aggregates, cursor) = load_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
        )
        .await
        .expect("load v2 projection")
        .expect("complete v2 minute coverage");
        let minute = start.timestamp();
        let aggregate = aggregates.get(&minute).expect("first minute aggregate");
        assert_eq!(cursor, 18);
        assert_eq!(aggregate.total_count, 2);
        assert_eq!(aggregate.first_byte_ttfb_values, vec![10.0, 100.0]);
        assert_eq!(aggregate.first_token_values, vec![11.0, 101.0]);
        assert_eq!(aggregate.first_byte_p95_ms(), Some(95.5));
    }

    #[tokio::test]
    async fn minute_projection_v2_warms_account_scope_with_empty_minutes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2 (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, aggregate_json TEXT NOT NULL, total_latency_samples_json TEXT NOT NULL, first_byte_samples_json TEXT NOT NULL, first_response_byte_total_samples_json TEXT NOT NULL, first_token_samples_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, coverage_state TEXT NOT NULL DEFAULT 'warming', updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection table");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection state table");
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, last_error) VALUES ('timeseries_minute_v2', 'ready')",
        )
        .execute(&pool)
        .await
        .expect("mark v2 projection ready");
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
        let end = start + ChronoDuration::minutes(2);

        store_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            Some(42),
            &[record(17, "2026-08-01 08:00:12")],
        )
        .await
        .expect("warm account projection");

        let (aggregates, cursor) = load_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            Some(42),
        )
        .await
        .expect("load account projection")
        .expect("complete account minute coverage");
        assert_eq!(cursor, 17);
        assert_eq!(aggregates.len(), 2);
        assert_eq!(
            aggregates[&(start + ChronoDuration::minutes(1)).timestamp()].total_count,
            0
        );
    }

    #[tokio::test]
    async fn minute_projection_warm_snapshot_does_not_replace_a_newer_cursor() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2 (minute_start_epoch INTEGER NOT NULL, source_scope TEXT NOT NULL, upstream_account_key INTEGER NOT NULL, aggregate_json TEXT NOT NULL, total_latency_samples_json TEXT NOT NULL, first_byte_samples_json TEXT NOT NULL, first_response_byte_total_samples_json TEXT NOT NULL, first_token_samples_json TEXT NOT NULL, max_row_id INTEGER NOT NULL DEFAULT 0, coverage_state TEXT NOT NULL DEFAULT 'warming', updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY (minute_start_epoch, source_scope, upstream_account_key))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection table");
        sqlx::query(
            "CREATE TABLE timeseries_minute_projection_v2_state (consumer TEXT PRIMARY KEY, cursor_row_id INTEGER NOT NULL DEFAULT 0, last_flush_at TEXT, last_error TEXT, updated_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&pool)
        .await
        .expect("create v2 projection state table");
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2_state (consumer, last_error) VALUES ('timeseries_minute_v2', 'ready')",
        )
        .execute(&pool)
        .await
        .expect("mark v2 projection ready");
        let start = Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).single().unwrap();
        let end = start + ChronoDuration::minutes(1);
        let old = record(17, "2026-08-01 08:00:12");
        let newer = record(18, "2026-08-01 08:00:30");

        store_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &[old.clone(), newer],
        )
        .await
        .expect("store newer snapshot");
        store_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &[old],
        )
        .await
        .expect("attempt stale warm snapshot");

        let (aggregates, cursor) = load_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
        )
        .await
        .expect("load v2 projection")
        .expect("complete minute coverage");
        assert_eq!(cursor, 18);
        assert_eq!(aggregates[&start.timestamp()].total_count, 2);

        sqlx::query("UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming'")
            .execute(&pool)
            .await
            .expect("invalidate stale projection coverage");
        assert!(
            load_timeseries_minute_projection_v2(
                &pool,
                start,
                end,
                InvocationSourceScope::All,
                None,
            )
            .await
            .expect("load invalidated projection")
            .is_none()
        );
        store_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
            &[
                record(17, "2026-08-01 08:00:12"),
                record(18, "2026-08-01 08:00:30"),
            ],
        )
        .await
        .expect("re-warm invalidated projection");
        let (aggregates, cursor) = load_timeseries_minute_projection_v2(
            &pool,
            start,
            end,
            InvocationSourceScope::All,
            None,
        )
        .await
        .expect("load re-warmed projection")
        .expect("re-warmed minute coverage");
        assert_eq!(cursor, 18);
        assert_eq!(aggregates[&start.timestamp()].total_count, 2);
    }

    #[test]
    fn minute_projection_folds_exact_samples_into_the_requested_reporting_bucket() {
        let mut minute_aggregates = BTreeMap::new();
        for minute in [0_i64, 60, 120, 180, 240] {
            let mut aggregate = BucketAggregate {
                total_count: 1,
                first_byte_sample_count: 1,
                ..Default::default()
            };
            aggregate.first_byte_ttfb_values.push(minute as f64);
            minute_aggregates.insert(minute, aggregate);
        }

        let folded =
            fold_minute_projection_aggregates(minute_aggregates, 300, chrono_tz::Asia::Shanghai)
                .expect("fold minute aggregates");
        assert_eq!(folded.len(), 1);
        let aggregate = folded.get(&0).expect("five-minute bucket");
        assert_eq!(aggregate.total_count, 5);
        assert_eq!(
            aggregate.first_byte_ttfb_values,
            vec![0.0, 60.0, 120.0, 180.0, 240.0]
        );
    }
}

pub(crate) async fn fetch_timeseries(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let bucket_selection = resolve_timeseries_bucket_selection(
        &params,
        &range_window,
        state.config.invocation_max_days,
    )?;
    if let Some(upstream_account_id) = params.upstream_account_id {
        return fetch_timeseries_for_account(
            state,
            reporting_tz,
            source_scope,
            range_window,
            bucket_selection,
            upstream_account_id,
        )
        .await;
    }
    let bucket_seconds = bucket_selection.bucket_seconds;

    if bucket_seconds >= 3_600 {
        let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, &range_window);
        let needs_historical_rollups =
            range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days);
        if !tz_is_hour_aligned {
            if needs_historical_rollups {
                return Err(ApiError::bad_request(anyhow!(
                    "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
                )));
            }
        } else {
            return fetch_timeseries_from_hourly_rollups(
                state,
                params,
                reporting_tz,
                source_scope,
                range_window,
                bucket_selection,
            )
            .await;
        }
    }

    let end_dt = range_window.end;
    let start_dt = range_window.start;
    let start_str_iso = format_utc_iso(start_dt);
    let use_minute_projection = range_window.duration <= ChronoDuration::days(1);

    let coverage_invalidation_pending = state
        .terminal_projection_hub
        .timeseries_coverage_invalidation_pending()
        .is_some();
    if use_minute_projection
        && !coverage_invalidation_pending
        && let Some((minute_aggregates, projection_cursor)) =
            load_timeseries_minute_projection_v2(&state.pool, start_dt, end_dt, source_scope, None)
                .await?
    {
        let mut aggregates =
            fold_minute_projection_aggregates(minute_aggregates, bucket_seconds, reporting_tz)?;
        let (full_minute_start_epoch, full_minute_end_epoch) =
            complete_minute_bounds(start_dt, end_dt);
        let full_minute_start = Utc
            .timestamp_opt(full_minute_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid timeseries full-minute start"))?;
        let full_minute_end = Utc
            .timestamp_opt(full_minute_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid timeseries full-minute end"))?;
        let mut exact_records = query_invocation_aggregate_records_from_live_range(
            &state.pool,
            ExactUtcRange {
                start: full_minute_start,
                end: full_minute_end,
            },
            source_scope,
            Some(projection_cursor),
            Some(snapshot_id),
        )
        .await?;
        for (boundary_start, boundary_end) in [
            (start_dt, end_dt.min(full_minute_start)),
            (start_dt.max(full_minute_end), end_dt),
        ] {
            if let Some(range) = exact_utc_range(boundary_start, boundary_end)? {
                exact_records.extend(
                    query_invocation_aggregate_records_from_live_range(
                        &state.pool,
                        range,
                        source_scope,
                        None,
                        Some(snapshot_id),
                    )
                    .await?,
                );
            }
        }
        let in_flight_records = query_in_flight_invocation_aggregate_records_from_live_range(
            &state.pool,
            ExactUtcRange {
                start: full_minute_start,
                end: full_minute_end,
            },
            source_scope,
            Some(snapshot_id),
        )
        .await?;
        let db_runtime_records = collect_in_flight_aggregate_records(&in_flight_records);
        let mut seen_ids = exact_records
            .iter()
            .map(|record| record.id)
            .collect::<HashSet<_>>();
        extend_unique_invocation_records(&mut exact_records, &mut seen_ids, in_flight_records);
        for record in exact_records {
            let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
                continue;
            };
            let bucket_epoch =
                align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
            add_exact_record_to_timeseries_aggregate(
                aggregates.entry(bucket_epoch).or_default(),
                &record,
            );
        }
        let memory_overlay_count = add_pending_timeseries_deltas(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            None,
            full_minute_start,
            full_minute_end,
            bucket_seconds,
            reporting_tz,
            Some(projection_cursor),
            snapshot_id,
        )?;
        let fill_start_epoch =
            align_reporting_bucket_epoch(start_dt.timestamp(), bucket_seconds, reporting_tz)?;
        let fill_end_epoch =
            resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
        let mut bucket_cursor = fill_start_epoch;
        while bucket_cursor < fill_end_epoch {
            aggregates.entry(bucket_cursor).or_default();
            bucket_cursor =
                next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
        }
        overlay_runtime_timeseries_in_flight(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            None,
            start_dt,
            end_dt,
            bucket_seconds,
            reporting_tz,
            &db_runtime_records,
        )?;
        debug!(
            route = "timeseries_http_or_topic",
            builder = "minute_projection_v2",
            response_source = "minute_projection",
            minute_rollup_count = aggregates.len(),
            memory_overlay_count,
            exact_fallback_minute_count = 2_u8,
            raw_row_count = db_runtime_records.len(),
            coverage_state = "covered",
            projection_cursor,
            "built open-window timeseries from minute projection"
        );
        return build_timeseries_response(
            start_dt,
            end_dt,
            bucket_seconds,
            snapshot_id,
            bucket_selection,
            aggregates,
            fill_start_epoch,
            fill_end_epoch,
            reporting_tz,
        );
    }

    let (records, response_source) = if use_minute_projection {
        let records = query_invocation_aggregate_records_from_live_range(
            &state.pool,
            ExactUtcRange {
                start: start_dt,
                end: end_dt,
            },
            source_scope,
            None,
            Some(snapshot_id),
        )
        .await?;
        // Projection persistence is P2 work. Never make a read request wait for a SQLite
        // writer when a first-time exact fallback already produced a valid response.
        let projection_pool = state.pool.clone();
        let projection_records = records.clone();
        let projection_snapshot_records = projection_records
            .iter()
            .filter(|record| {
                !prompt_shared::invocation_status_is_in_flight(record.status.as_deref())
            })
            .map(timeseries_projection_snapshot_record)
            .collect::<Vec<_>>();
        let terminal_projection_hub = state.terminal_projection_hub.clone();
        let projection_selection = TimeseriesProjectionSelection {
            source_scope: timeseries_projection_scope(source_scope),
            upstream_account_id: None,
        };
        tokio::spawn(async move {
            if let Err(error) = store_timeseries_minute_projection_v2(
                &projection_pool,
                start_dt,
                end_dt,
                source_scope,
                None,
                &projection_records,
            )
            .await
            {
                debug!(
                    route = "timeseries_projection",
                    projection_store_outcome = "deferred_failed",
                    ?error,
                    "v2 minute projection warm write failed"
                );
            } else {
                terminal_projection_hub.mark_timeseries_warm_coverage(
                    projection_selection,
                    &projection_snapshot_records,
                );
            }
        });
        (
            records,
            if coverage_invalidation_pending {
                "exact_fallback_projection_invalidated"
            } else {
                "exact_fallback"
            },
        )
    } else {
        (
            query_invocation_aggregate_records_from_live_range(
                &state.pool,
                ExactUtcRange {
                    start: start_dt,
                    end: end_dt,
                },
                source_scope,
                None,
                Some(snapshot_id),
            )
            .await?,
            "exact_range",
        )
    };
    debug!(
        route = "timeseries_http_or_topic",
        builder = "minute_projection_v2",
        response_source,
        raw_row_count = records.len(),
        coverage_state = if response_source == "minute_projection" {
            "covered"
        } else {
            "warming"
        },
        "built open-window timeseries"
    );
    let db_runtime_records = collect_in_flight_aggregate_records(&records);

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    let start_epoch = start_dt.timestamp();

    for record in records {
        let naive = NaiveDateTime::parse_from_str(&record.occurred_at, "%Y-%m-%d %H:%M:%S")
            .map_err(|err| anyhow!("failed to parse occurred_at: {err}"))?;
        // Interpret stored naive time as local Asia/Shanghai and convert to UTC epoch
        let epoch = Shanghai
            .from_local_datetime(&naive)
            .single()
            .map(|dt| dt.with_timezone(&Utc).timestamp())
            .unwrap_or_else(|| naive.and_utc().timestamp());
        let bucket_epoch = align_reporting_bucket_epoch(epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        let classification = resolve_failure_classification(
            record.status.as_deref(),
            record.error_message.as_deref(),
            record.failure_kind.as_deref(),
            record.failure_class.as_deref(),
            record.is_actionable,
        );
        let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
            record.status.as_deref(),
            record.error_message.as_deref(),
        ) && classification.failure_class == FailureClass::None;
        if is_success_like {
            entry.success_count += 1;
        } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            entry.in_flight_count += 1;
            entry
                .in_flight_phase_counts
                .increment_phase_name(record.live_phase.as_deref());
        } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
            record.status.as_deref(),
        ) && classification.failure_class != FailureClass::None
        {
            entry.failure_count += 1;
        }
        let latency_status = if is_success_like {
            Some("success")
        } else {
            record.status.as_deref()
        };
        if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            entry.record_total_latency_sample(record.t_total_ms);
        }
        entry.record_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.record_first_token_sample(record.first_token_ms);
        entry.total_tokens += record.total_tokens.unwrap_or(0);
        entry.cache_input_tokens += record.cache_input_tokens.unwrap_or(0);
        let cost = record.cost.unwrap_or(0.0);
        entry.total_cost += cost;
        if invocation_counts_toward_non_success_usage(
            record.status.as_deref(),
            record.error_message.as_deref(),
            record.failure_kind.as_deref(),
            record.failure_class.as_deref(),
            record.is_actionable,
        ) {
            entry.non_success_cost += cost;
        }
    }

    // Fill every bucket that intersects the requested range using reporting-timezone
    // boundaries rather than fixed UTC-duration strides. This keeps DST transition
    // days aligned to local clock buckets.
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }
    overlay_runtime_timeseries_in_flight(
        state.as_ref(),
        &mut aggregates,
        source_scope,
        None,
        start_dt,
        end_dt,
        bucket_seconds,
        reporting_tz,
        &db_runtime_records,
    )?;

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        // Skip any buckets outside the desired window. This guards against
        // future-dated records leaking past the clamped end.
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        points.push(timeseries_point_from_aggregate(start, end, &agg));
    }

    let response = TimeseriesResponse {
        range_start: start_str_iso,
        range_end: format_utc_iso(end_dt),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    };

    Ok(Json(response))
}

pub(crate) async fn fetch_timeseries_for_account(
    state: Arc<AppState>,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
    upstream_account_id: i64,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    let start_dt = range_window.start;
    let end_dt = range_window.end;
    let start_epoch = start_dt.timestamp();
    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();

    if bucket_seconds >= 3_600 {
        let tz_is_hour_aligned = reporting_tz_has_whole_hour_offsets(reporting_tz, &range_window);
        let needs_historical_rollups =
            range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days);
        if !tz_is_hour_aligned && needs_historical_rollups {
            return Err(ApiError::bad_request(anyhow!(
                "unsupported timeZone for historical hourly timeseries: {reporting_tz}; historical hourly buckets require whole-hour UTC offsets"
            )));
        }
    }

    let coverage_invalidation_pending = state
        .terminal_projection_hub
        .timeseries_coverage_invalidation_pending()
        .is_some();
    if bucket_seconds < 3_600
        && range_window.duration <= ChronoDuration::days(1)
        && !coverage_invalidation_pending
        && let Some((minute_aggregates, projection_cursor)) = load_timeseries_minute_projection_v2(
            &state.pool,
            start_dt,
            end_dt,
            source_scope,
            Some(upstream_account_id),
        )
        .await?
    {
        let mut aggregates =
            fold_minute_projection_aggregates(minute_aggregates, bucket_seconds, reporting_tz)?;
        let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
        let (full_minute_start_epoch, full_minute_end_epoch) =
            complete_minute_bounds(start_dt, end_dt);
        let full_minute_start = Utc
            .timestamp_opt(full_minute_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid account timeseries full-minute start"))?;
        let full_minute_end = Utc
            .timestamp_opt(full_minute_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid account timeseries full-minute end"))?;
        let mut exact_records = query_invocation_aggregate_records_from_live_range_for_account(
            &state.pool,
            ExactUtcRange {
                start: full_minute_start,
                end: full_minute_end,
            },
            source_scope,
            Some(projection_cursor),
            Some(snapshot_id),
            upstream_account_id,
        )
        .await?;
        for (boundary_start, boundary_end) in [
            (start_dt, end_dt.min(full_minute_start)),
            (start_dt.max(full_minute_end), end_dt),
        ] {
            if let Some(range) = exact_utc_range(boundary_start, boundary_end)? {
                exact_records.extend(
                    query_invocation_aggregate_records_from_live_range_for_account(
                        &state.pool,
                        range,
                        source_scope,
                        None,
                        Some(snapshot_id),
                        upstream_account_id,
                    )
                    .await?,
                );
            }
        }
        let in_flight_records =
            query_in_flight_invocation_aggregate_records_from_live_range_for_account(
                &state.pool,
                ExactUtcRange {
                    start: full_minute_start,
                    end: full_minute_end,
                },
                source_scope,
                Some(snapshot_id),
                upstream_account_id,
            )
            .await?;
        let db_runtime_records = collect_in_flight_aggregate_records(&in_flight_records);
        let mut seen_ids = exact_records
            .iter()
            .map(|record| record.id)
            .collect::<HashSet<_>>();
        extend_unique_invocation_records(&mut exact_records, &mut seen_ids, in_flight_records);
        for record in exact_records {
            let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
                continue;
            };
            let bucket_epoch =
                align_reporting_bucket_epoch(occurred.timestamp(), bucket_seconds, reporting_tz)?;
            add_exact_record_to_timeseries_aggregate(
                aggregates.entry(bucket_epoch).or_default(),
                &record,
            );
        }
        let memory_overlay_count = add_pending_timeseries_deltas(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            Some(upstream_account_id),
            full_minute_start,
            full_minute_end,
            bucket_seconds,
            reporting_tz,
            Some(projection_cursor),
            snapshot_id,
        )?;
        let fill_start_epoch =
            align_reporting_bucket_epoch(start_dt.timestamp(), bucket_seconds, reporting_tz)?;
        let fill_end_epoch =
            resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
        let mut bucket_cursor = fill_start_epoch;
        while bucket_cursor < fill_end_epoch {
            aggregates.entry(bucket_cursor).or_default();
            bucket_cursor =
                next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
        }
        overlay_runtime_timeseries_in_flight(
            state.as_ref(),
            &mut aggregates,
            source_scope,
            Some(upstream_account_id),
            start_dt,
            end_dt,
            bucket_seconds,
            reporting_tz,
            &db_runtime_records,
        )?;
        debug!(
            route = "timeseries_http_or_topic",
            builder = "minute_projection_v2",
            response_source = "minute_projection",
            upstream_account_id,
            minute_rollup_count = aggregates.len(),
            memory_overlay_count,
            exact_fallback_minute_count = 2_u8,
            raw_row_count = db_runtime_records.len(),
            coverage_state = "covered",
            projection_cursor,
            "built account open-window timeseries from minute projection"
        );
        return build_timeseries_response(
            start_dt,
            end_dt,
            bucket_seconds,
            snapshot_id,
            bucket_selection,
            aggregates,
            fill_start_epoch,
            fill_end_epoch,
            reporting_tz,
        );
    }

    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch = resolve_timeseries_fill_end_epoch(end_dt, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
    if bucket_seconds < 3_600 && range_window.duration <= ChronoDuration::days(1) {
        // Account projections include empty minutes, so a first exact fallback must warm
        // the complete selection instead of relying on future terminal events to fill gaps.
        let projection_pool = state.pool.clone();
        let terminal_projection_hub = state.terminal_projection_hub.clone();
        let projection_selection = TimeseriesProjectionSelection {
            source_scope: timeseries_projection_scope(source_scope),
            upstream_account_id: Some(upstream_account_id),
        };
        tokio::spawn(async move {
            let records = query_invocation_aggregate_records_from_live_range_for_account(
                &projection_pool,
                ExactUtcRange {
                    start: start_dt,
                    end: end_dt,
                },
                source_scope,
                None,
                Some(snapshot_id),
                upstream_account_id,
            )
            .await;
            let result = match records {
                Ok(records) => {
                    let projection_snapshot_records = records
                        .iter()
                        .filter(|record| {
                            !prompt_shared::invocation_status_is_in_flight(record.status.as_deref())
                        })
                        .map(timeseries_projection_snapshot_record)
                        .collect::<Vec<_>>();
                    store_timeseries_minute_projection_v2(
                        &projection_pool,
                        start_dt,
                        end_dt,
                        source_scope,
                        Some(upstream_account_id),
                        &records,
                    )
                    .await
                    .map(|()| projection_snapshot_records)
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(projection_snapshot_records) => {
                    terminal_projection_hub.mark_timeseries_warm_coverage(
                        projection_selection,
                        &projection_snapshot_records,
                    );
                }
                Err(error) => {
                    debug!(
                        route = "timeseries_projection",
                        upstream_account_id,
                        projection_store_outcome = "deferred_failed",
                        ?error,
                        "account v2 minute projection warm write failed"
                    );
                }
            }
        });
    }
    let mut db_runtime_records = HashMap::new();
    let range_plan = if bucket_seconds >= 3_600 {
        build_hourly_rollup_exact_range_plan(
            start_dt,
            end_dt,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?
    } else {
        let rollup_bucket_seconds = 60;
        let range_start_epoch = if start_dt.timestamp().rem_euclid(rollup_bucket_seconds) == 0 {
            start_dt.timestamp()
        } else {
            align_bucket_epoch(
                start_dt
                    .timestamp()
                    .saturating_add(rollup_bucket_seconds.saturating_sub(1)),
                rollup_bucket_seconds,
                0,
            )
        };
        let range_end_epoch = align_bucket_epoch(end_dt.timestamp(), rollup_bucket_seconds, 0);
        let mut live_exact_ranges = Vec::new();
        let first_full_bucket_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid first full bucket start epoch"))?;
        let last_full_bucket_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid last full bucket end epoch"))?;
        push_exact_range(
            &mut live_exact_ranges,
            start_dt,
            end_dt.min(first_full_bucket_start),
        )?;
        push_exact_range(
            &mut live_exact_ranges,
            start_dt.max(last_full_bucket_end),
            end_dt,
        )?;
        HourlyRollupExactRangePlan {
            full_hour_range: (range_start_epoch < range_end_epoch)
                .then_some((range_start_epoch, range_end_epoch)),
            live_exact_ranges,
        }
    };
    let mut tx = state.pool.begin().await?;
    let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
    if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
        let table_name = if bucket_seconds >= 3_600 {
            "upstream_account_stats_hourly"
        } else {
            "upstream_account_stats_minute"
        };
        let rows = query_upstream_account_stats_rollup_range_tx(
            tx.as_mut(),
            table_name,
            range_start_epoch,
            range_end_epoch,
            source_scope,
            upstream_account_id,
        )
        .await?;
        add_rollup_rows_to_timeseries_aggregates(
            &mut aggregates,
            rows,
            bucket_seconds,
            reporting_tz,
        )?;
    }

    let boundary_snapshot_id = rollup_live_cursor.min(snapshot_id);
    if !range_plan.live_exact_ranges.is_empty() && boundary_snapshot_id > 0 {
        let exact_records = query_invocation_exact_records_for_account_tx(
            tx.as_mut(),
            &range_plan,
            source_scope,
            boundary_snapshot_id,
            upstream_account_id,
        )
        .await?;
        db_runtime_records.extend(collect_in_flight_aggregate_records(&exact_records));
        add_exact_records_to_timeseries_aggregates(
            &mut aggregates,
            exact_records,
            bucket_seconds,
            reporting_tz,
        )?;
    }

    let mut archive_overlap_ids = HashSet::new();
    if rollup_live_cursor < snapshot_id {
        let tail_range_plan = HourlyRollupExactRangePlan {
            full_hour_range: None,
            live_exact_ranges: exact_utc_range(start_dt, end_dt)?.into_iter().collect(),
        };
        let tail_records = query_invocation_exact_records_tx_for_account(
            tx.as_mut(),
            &tail_range_plan,
            source_scope,
            snapshot_id,
            upstream_account_id,
            rollup_live_cursor,
        )
        .await?;
        archive_overlap_ids.extend(tail_records.iter().map(|record| record.id));
        db_runtime_records.extend(collect_in_flight_aggregate_records(&tail_records));
        add_exact_records_to_timeseries_aggregates(
            &mut aggregates,
            tail_records,
            bucket_seconds,
            reporting_tz,
        )?;
    }
    if bucket_seconds >= 3_600
        && let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range
    {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries start epoch"))
            })?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| {
                ApiError::from(anyhow!("invalid account archived timeseries end epoch"))
            })?;
        let archived_rows =
            crate::stats::query_unmaterialized_upstream_account_archive_hourly_rollup_deltas(
                &state.pool,
                HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
                source_scope,
                Some((archived_start, archived_end)),
                Some(&archive_overlap_ids),
                upstream_account_id,
            )
            .await?;
        for row in archived_rows {
            let bucket_epoch =
                align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
            if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
                entry.total_count += row.total_count;
                entry.success_count += row.success_count;
                entry.failure_count += row.failure_count;
                entry.in_flight_count += row.in_flight_count;
                entry.total_tokens += row.total_tokens;
                entry.cache_input_tokens += row.cache_input_tokens;
                entry.total_cost += row.total_cost;
                entry.non_success_cost += row.non_success_cost;
                entry.total_latency_sample_count += row.total_latency_sample_count;
                entry.total_latency_sum_ms += row.total_latency_sum_ms;
                entry.first_byte_sample_count += row.first_byte_sample_count;
                entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
                entry.first_byte_histogram = if entry.first_byte_histogram.is_empty() {
                    decode_approx_histogram(&row.first_byte_histogram)
                } else {
                    let mut merged = entry.first_byte_histogram.clone();
                    merge_approx_histogram_into(
                        &mut merged,
                        &decode_approx_histogram(&row.first_byte_histogram),
                    )?;
                    merged
                };
                entry.first_response_byte_total_sample_count +=
                    row.first_response_byte_total_sample_count;
                entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
                entry.first_response_byte_total_histogram =
                    if entry.first_response_byte_total_histogram.is_empty() {
                        decode_approx_histogram(&row.first_response_byte_total_histogram)
                    } else {
                        let mut merged = entry.first_response_byte_total_histogram.clone();
                        merge_approx_histogram_into(
                            &mut merged,
                            &decode_approx_histogram(&row.first_response_byte_total_histogram),
                        )?;
                        merged
                    };
                entry.first_token_sample_count += row.first_token_sample_count;
                entry.first_token_sum_ms += row.first_token_sum_ms;
                entry.first_token_histogram = if entry.first_token_histogram.is_empty() {
                    decode_approx_histogram(&row.first_token_histogram)
                } else {
                    let mut merged = entry.first_token_histogram.clone();
                    merge_approx_histogram_into(
                        &mut merged,
                        &decode_approx_histogram(&row.first_token_histogram),
                    )?;
                    merged
                };
            }
        }
    }
    overlay_runtime_timeseries_in_flight(
        state.as_ref(),
        &mut aggregates,
        source_scope,
        Some(upstream_account_id),
        start_dt,
        end_dt,
        bucket_seconds,
        reporting_tz,
        &db_runtime_records,
    )?;
    drop(tx);
    build_timeseries_response(
        start_dt,
        end_dt,
        bucket_seconds,
        snapshot_id,
        bucket_selection,
        aggregates,
        fill_start_epoch,
        fill_end_epoch,
        reporting_tz,
    )
}

pub(crate) fn add_rollup_rows_to_timeseries_aggregates(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    rows: Vec<UpstreamAccountStatsRollupRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for row in rows {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += row.total_count;
            entry.success_count += row.success_count;
            entry.failure_count += row.failure_count;
            entry.in_flight_count += row.in_flight_count;
            entry.total_tokens += row.total_tokens;
            entry.cache_input_tokens += row.cache_input_tokens;
            entry.total_cost += row.total_cost;
            entry.non_success_cost += row.non_success_cost;
            entry.total_latency_sample_count += row.total_latency_sample_count;
            entry.total_latency_sum_ms += row.total_latency_sum_ms;
            entry.first_byte_sample_count += row.first_byte_sample_count;
            entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
            entry.first_byte_histogram = if entry.first_byte_histogram.is_empty() {
                decode_approx_histogram(&row.first_byte_histogram)
            } else {
                let mut merged = entry.first_byte_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_byte_histogram),
                )?;
                merged
            };
            entry.first_response_byte_total_sample_count +=
                row.first_response_byte_total_sample_count;
            entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
            entry.first_response_byte_total_histogram =
                if entry.first_response_byte_total_histogram.is_empty() {
                    decode_approx_histogram(&row.first_response_byte_total_histogram)
                } else {
                    let mut merged = entry.first_response_byte_total_histogram.clone();
                    merge_approx_histogram_into(
                        &mut merged,
                        &decode_approx_histogram(&row.first_response_byte_total_histogram),
                    )?;
                    merged
                };
            entry.first_token_sample_count += row.first_token_sample_count;
            entry.first_token_sum_ms += row.first_token_sum_ms;
            entry.first_token_histogram = if entry.first_token_histogram.is_empty() {
                decode_approx_histogram(&row.first_token_histogram)
            } else {
                let mut merged = entry.first_token_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_token_histogram),
                )?;
                merged
            };
        }
    }
    Ok(())
}

pub(crate) fn add_exact_records_to_timeseries_aggregates(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    records: Vec<InvocationAggregateRecord>,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<(), ApiError> {
    for record in records {
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            add_exact_record_to_timeseries_aggregate(entry, &record);
        }
    }
    Ok(())
}

pub(crate) fn add_exact_record_to_timeseries_aggregate(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) {
    entry.total_count += 1;
    let classification = resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    );
    let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
        record.status.as_deref(),
        record.error_message.as_deref(),
    ) && classification.failure_class == FailureClass::None;
    if is_success_like {
        entry.success_count += 1;
    } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        entry.in_flight_count += 1;
        entry
            .in_flight_phase_counts
            .increment_phase_name(record.live_phase.as_deref());
    } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
        record.status.as_deref(),
    ) && classification.failure_class != FailureClass::None
    {
        entry.failure_count += 1;
    }
    let latency_status = if is_success_like {
        Some("success")
    } else {
        record.status.as_deref()
    };
    if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        entry.record_total_latency_sample(record.t_total_ms);
    }
    entry.record_exact_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
    entry.record_exact_first_response_byte_total_sample(
        record.t_req_read_ms,
        record.t_req_parse_ms,
        record.t_upstream_connect_ms,
        record.t_upstream_ttfb_ms,
    );
    entry.record_first_token_sample(record.first_token_ms);
    entry.total_tokens += record.total_tokens.unwrap_or_default();
    entry.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
    let cost = record.cost.unwrap_or_default();
    entry.total_cost += cost;
    if invocation_counts_toward_non_success_usage(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable,
    ) {
        entry.non_success_cost += cost;
    }
}

pub(crate) fn subtract_stale_in_flight_record_from_timeseries_aggregate(
    entry: &mut BucketAggregate,
    record: &InvocationAggregateRecord,
) {
    if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
        return;
    }
    entry.total_count = entry.total_count.saturating_sub(1);
    entry.in_flight_count = entry.in_flight_count.saturating_sub(1);
    entry
        .in_flight_phase_counts
        .decrement_phase_name(record.live_phase.as_deref());
    entry.total_tokens = entry
        .total_tokens
        .saturating_sub(record.total_tokens.unwrap_or_default());
    entry.cache_input_tokens = entry
        .cache_input_tokens
        .saturating_sub(record.cache_input_tokens.unwrap_or_default());
    entry.total_cost = (entry.total_cost - record.cost.unwrap_or_default()).max(0.0);
    entry.remove_exact_first_response_byte_total_sample(
        record.t_req_read_ms,
        record.t_req_parse_ms,
        record.t_upstream_connect_ms,
        record.t_upstream_ttfb_ms,
    );
    entry.remove_exact_first_token_sample(record.first_token_ms);
}

pub(crate) fn overlay_runtime_timeseries_in_flight(
    state: &AppState,
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    db_runtime_records: &HashMap<(String, String), InvocationAggregateRecord>,
) -> Result<(), ApiError> {
    let mut runtime_overlay_row_count = 0_i64;
    let mut stale_db_runtime_row_count = 0_i64;
    for record in state.proxy_runtime_invocations.snapshot() {
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if let Some(expected_upstream_account_id) = upstream_account_id
            && record.upstream_account_id != Some(expected_upstream_account_id)
        {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        if occurred_utc < start_dt || occurred_utc >= end_dt {
            if let Some(db_record) = db_runtime_records.get(&key) {
                subtract_stale_db_runtime_record(
                    aggregates,
                    db_record,
                    bucket_seconds,
                    reporting_tz,
                    &mut stale_db_runtime_row_count,
                )?;
            }
            continue;
        }
        if let Some(db_record) = db_runtime_records.get(&key) {
            subtract_stale_db_runtime_record(
                aggregates,
                db_record,
                bucket_seconds,
                reporting_tz,
                &mut stale_db_runtime_row_count,
            )?;
        }
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += 1;
        entry.in_flight_count += 1;
        let runtime_phase = record
            .live_phase
            .as_deref()
            .or_else(|| runtime_invocation_live_phase(&record));
        entry
            .in_flight_phase_counts
            .increment_phase_name(runtime_phase);
        entry.record_ttfb_sample(record.status.as_deref(), record.t_upstream_ttfb_ms);
        entry.record_first_response_byte_total_sample(
            record.t_req_read_ms,
            record.t_req_parse_ms,
            record.t_upstream_connect_ms,
            record.t_upstream_ttfb_ms,
        );
        entry.record_first_token_sample(record.first_token_ms);
        entry.total_tokens += record.total_tokens.unwrap_or_default();
        entry.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
        entry.total_cost += record.cost.unwrap_or_default();
        runtime_overlay_row_count += 1;
    }
    if runtime_overlay_row_count > 0 || stale_db_runtime_row_count > 0 {
        debug!(
            endpoint = "/api/timeseries",
            runtime_overlay_row_count,
            stale_db_runtime_row_count,
            upstream_account_id,
            "overlayed memory runtime in-flight records into timeseries"
        );
    }
    Ok(())
}

pub(crate) fn subtract_stale_db_runtime_record(
    aggregates: &mut BTreeMap<i64, BucketAggregate>,
    record: &InvocationAggregateRecord,
    bucket_seconds: i64,
    reporting_tz: Tz,
    stale_db_runtime_row_count: &mut i64,
) -> Result<(), ApiError> {
    let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
        return Ok(());
    };
    let bucket_epoch =
        align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
    if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
        subtract_stale_in_flight_record_from_timeseries_aggregate(entry, record);
        *stale_db_runtime_row_count += 1;
    }
    Ok(())
}

pub(crate) fn collect_in_flight_aggregate_records(
    records: &[InvocationAggregateRecord],
) -> HashMap<(String, String), InvocationAggregateRecord> {
    records
        .iter()
        .filter(|record| prompt_shared::invocation_status_is_in_flight(record.status.as_deref()))
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record.clone(),
            )
        })
        .collect()
}

pub(crate) fn timeseries_point_from_aggregate(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    agg: &BucketAggregate,
) -> TimeseriesPoint {
    let has_calls = agg
        .total_count
        .max(agg.success_count + agg.failure_count + agg.in_flight_count.max(0))
        > 0;
    TimeseriesPoint {
        bucket_start: format_utc_iso(start),
        bucket_end: format_utc_iso(end),
        total_count: agg.total_count,
        success_count: agg.success_count,
        failure_count: agg.failure_count,
        in_flight_count: agg.in_flight_count,
        in_flight_phase_counts: agg.in_flight_phase_counts,
        total_tokens: agg.total_tokens,
        cache_input_tokens: agg.cache_input_tokens,
        total_cost: agg.total_cost,
        non_success_cost: agg.non_success_cost,
        avg_total_ms: has_calls.then(|| agg.total_latency_avg_ms()).flatten(),
        total_latency_sample_count: if has_calls {
            agg.total_latency_sample_count
        } else {
            0
        },
        first_byte_sample_count: if has_calls {
            agg.first_byte_sample_count
        } else {
            0
        },
        first_byte_avg_ms: has_calls.then(|| agg.first_byte_avg_ms()).flatten(),
        first_byte_p95_ms: has_calls.then(|| agg.first_byte_p95_ms()).flatten(),
        first_response_byte_total_sample_count: if has_calls {
            agg.first_response_byte_total_sample_count
        } else {
            0
        },
        first_response_byte_total_avg_ms: has_calls
            .then(|| agg.first_response_byte_total_avg_ms())
            .flatten(),
        first_response_byte_total_p95_ms: has_calls
            .then(|| agg.first_response_byte_total_p95_ms())
            .flatten(),
        first_token_sample_count: if has_calls {
            agg.first_token_sample_count
        } else {
            0
        },
        first_token_avg_ms: has_calls.then(|| agg.first_token_avg_ms()).flatten(),
        first_token_p95_ms: has_calls.then(|| agg.first_token_p95_ms()).flatten(),
    }
}

pub(crate) fn build_timeseries_response(
    start_dt: DateTime<Utc>,
    end_dt: DateTime<Utc>,
    bucket_seconds: i64,
    snapshot_id: i64,
    bucket_selection: TimeseriesBucketSelection,
    aggregates: BTreeMap<i64, BucketAggregate>,
    fill_start_epoch: i64,
    fill_end_epoch: i64,
    reporting_tz: Tz,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        points.push(timeseries_point_from_aggregate(start, end, &agg));
    }

    Ok(Json(TimeseriesResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(end_dt),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}

#[cfg(test)]
pub(crate) async fn fetch_parallel_work_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Json<ParallelWorkStatsResponse>, ApiError> {
    load_parallel_work_stats_response(&state, params)
        .await
        .map(Json)
}

pub(crate) async fn fetch_parallel_work_stats_cached(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<ParallelWorkStatsQuery>,
) -> Result<Response, ApiError> {
    let response = load_parallel_work_stats_response(&state, params).await?;
    let body = serde_json::to_vec(&response)
        .map_err(|err| ApiError::from(anyhow!("failed to serialize parallel-work stats: {err}")))?;
    let etag = parallel_work_stats_etag(&body);
    let mut response = if request_etag_matches(&headers, &etag) {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response()
    };
    let etag_value = HeaderValue::from_str(&etag)
        .map_err(|err| ApiError::from(anyhow!("invalid parallel-work etag: {err}")))?;
    response
        .headers_mut()
        .insert(axum::http::header::ETAG, etag_value);
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    );
    Ok(response)
}

pub(crate) fn parallel_work_stats_etag(body: &[u8]) -> String {
    let digest = Sha256::digest(body);
    format!("\"parallel-work-{digest:x}\"")
}

pub(crate) fn request_etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(axum::http::header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag)
        })
        .unwrap_or(false)
}

pub(crate) async fn load_parallel_work_stats_response(
    state: &Arc<AppState>,
    params: ParallelWorkStatsQuery,
) -> Result<ParallelWorkStatsResponse, ApiError> {
    let requested_reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let upstream_account_id = params.upstream_account_id;
    let requested_range_window = resolve_range_window(&params.range, requested_reporting_tz)?;
    let bucket_params = TimeseriesQuery {
        range: params.range.clone(),
        bucket: params.bucket.clone(),
        settlement_hour: None,
        time_zone: params.time_zone.clone(),
        upstream_account_id,
    };
    let bucket_selection = resolve_timeseries_bucket_selection(
        &bucket_params,
        &requested_range_window,
        state.config.invocation_max_days,
    )?;
    let bucket_seconds = bucket_selection.bucket_seconds;
    let (reporting_tz, time_zone_fallback) = if bucket_seconds >= 3_600 {
        resolve_parallel_work_rollup_reporting_tz(requested_reporting_tz, &requested_range_window)
    } else {
        (requested_reporting_tz, false)
    };
    let range_window = if time_zone_fallback {
        resolve_range_window(&params.range, reporting_tz)?
    } else {
        requested_range_window
    };
    let fill_start_epoch =
        align_reporting_bucket_epoch(range_window.start.timestamp(), bucket_seconds, reporting_tz)?;
    let fill_end_epoch =
        resolve_timeseries_fill_end_epoch(range_window.end, bucket_seconds, reporting_tz)?;
    let fill_start = Utc
        .timestamp_opt(fill_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work fill start epoch")))?;
    let fill_end = Utc
        .timestamp_opt(fill_end_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work fill end epoch")))?;

    let current_counts = if bucket_seconds >= 3_600 {
        let leading_full_bucket_epoch = if fill_start < range_window.start {
            next_reporting_bucket_epoch(fill_start_epoch, bucket_seconds, reporting_tz)?
        } else {
            fill_start_epoch
        };
        let leading_full_bucket_start = Utc
            .timestamp_opt(leading_full_bucket_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid parallel-work rollup start epoch")))?;
        let mut bucket_keys = query_parallel_work_bucket_key_sets_from_hourly_rollups(
            &state.pool,
            leading_full_bucket_start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id,
        )
        .await?;
        let mut tx = state.pool.begin().await?;
        let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
        let rollup_live_cursor = load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
        drop(tx);
        if fill_start < range_window.start && range_window.start < leading_full_bucket_start {
            let leading_exact_end = leading_full_bucket_start.min(range_window.end);
            let leading_bucket_keys = query_parallel_work_exact_key_sets(
                &state.pool,
                range_window.start,
                leading_exact_end,
                bucket_seconds,
                reporting_tz,
                source_scope,
                upstream_account_id,
                None,
                Some(snapshot_id),
            )
            .await?;
            for (bucket_epoch, keys) in leading_bucket_keys {
                bucket_keys.entry(bucket_epoch).or_default().extend(keys);
            }
        }
        let tail_bucket_keys = query_parallel_work_exact_key_sets(
            &state.pool,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id,
            Some(rollup_live_cursor),
            Some(snapshot_id),
        )
        .await?;
        for (bucket_epoch, keys) in tail_bucket_keys {
            bucket_keys.entry(bucket_epoch).or_default().extend(keys);
        }
        parallel_work_counts_from_key_sets(bucket_keys)
    } else {
        parallel_work_counts_from_key_sets(
            query_parallel_work_exact_key_sets(
                &state.pool,
                range_window.start,
                range_window.end,
                bucket_seconds,
                reporting_tz,
                source_scope,
                upstream_account_id,
                None,
                None,
            )
            .await?,
        )
    };
    let conversations = if range_window.duration <= ChronoDuration::hours(24) {
        query_parallel_work_conversation_spans(
            &state.pool,
            range_window.start,
            range_window.end,
            bucket_seconds,
            reporting_tz,
            source_scope,
            upstream_account_id,
        )
        .await?
    } else {
        Vec::new()
    };
    let configured_full_detail_start_epoch =
        shanghai_retention_cutoff(state.config.invocation_success_full_days).timestamp();
    let active_minute_stats = query_parallel_work_active_minute_stats(
        &state.pool,
        range_window.start,
        range_window.end,
        source_scope,
        upstream_account_id,
        Some(
            load_parallel_work_full_detail_start_epoch(&state.pool)
                .await?
                .map(|persisted| persisted.max(configured_full_detail_start_epoch))
                .unwrap_or(configured_full_detail_start_epoch),
        ),
    )
    .await?;

    let current = build_parallel_work_window_response(
        fill_start,
        fill_end,
        bucket_seconds,
        reporting_tz,
        &current_counts,
        active_minute_stats,
        reporting_tz,
        time_zone_fallback,
        conversations,
    )?;

    Ok(ParallelWorkStatsResponse {
        current: current.clone(),
        minute7d: current.clone(),
        hour30d: current.clone(),
        day_all: current,
    })
}

pub(crate) async fn query_parallel_work_conversation_spans(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<Vec<ParallelWorkConversation>> {
    let mut query = QueryBuilder::new("SELECT ");
    query
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS conversation_id, MIN(occurred_at) AS first_occurred_at, MAX(occurred_at) AS last_occurred_at, COUNT(*) AS request_count FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(range_end))
        .push(" AND ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" IS NOT NULL AND ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" != ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    query
        .push(" GROUP BY ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" ORDER BY last_occurred_at DESC, request_count DESC LIMIT 80");

    let rows = query
        .build_query_as::<ParallelWorkConversationSpanRow>()
        .fetch_all(pool)
        .await?;
    let mut conversations = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(first_occurred_at) = parse_to_utc_datetime(&row.first_occurred_at) else {
            continue;
        };
        let Some(last_occurred_at) = parse_to_utc_datetime(&row.last_occurred_at) else {
            continue;
        };
        let start_epoch = align_reporting_bucket_epoch(
            first_occurred_at.timestamp(),
            bucket_seconds,
            reporting_tz,
        )?;
        let end_bucket_epoch = align_reporting_bucket_epoch(
            last_occurred_at.timestamp(),
            bucket_seconds,
            reporting_tz,
        )?;
        let end_epoch =
            next_reporting_bucket_epoch(end_bucket_epoch, bucket_seconds, reporting_tz)?;
        let start = Utc
            .timestamp_opt(start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid parallel-work conversation start epoch"))?;
        let end = Utc
            .timestamp_opt(end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid parallel-work conversation end epoch"))?;
        conversations.push(ParallelWorkConversation {
            conversation_id: row.conversation_id,
            start: format_utc_iso(start),
            end: format_utc_iso(end),
            request_count: row.request_count,
        });
    }

    Ok(conversations)
}

pub(crate) async fn fetch_timeseries_from_hourly_rollups(
    state: Arc<AppState>,
    _params: TimeseriesQuery,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    range_window: RangeWindow,
    bucket_selection: TimeseriesBucketSelection,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    let bucket_seconds = bucket_selection.bucket_seconds;
    let start_epoch = range_window.start.timestamp();
    let range_plan = build_hourly_rollup_exact_range_plan(
        range_window.start,
        range_window.end,
        shanghai_retention_cutoff(state.config.invocation_max_days),
    )?;

    let mut aggregates: BTreeMap<i64, BucketAggregate> = BTreeMap::new();
    let fill_start_epoch = align_reporting_bucket_epoch(start_epoch, bucket_seconds, reporting_tz)?;
    let fill_end_epoch =
        resolve_timeseries_fill_end_epoch(range_window.end, bucket_seconds, reporting_tz)?;
    let mut bucket_cursor = fill_start_epoch;
    while bucket_cursor < fill_end_epoch {
        aggregates.entry(bucket_cursor).or_default();
        bucket_cursor = next_reporting_bucket_epoch(bucket_cursor, bucket_seconds, reporting_tz)?;
    }

    let (snapshot_id, hourly_rows, exact_records, archive_overlap_ids) =
        if let Some((hourly_cursor, hourly_end_epoch)) = range_plan.full_hour_range {
            let mut tx = state.pool.begin().await?;
            let snapshot_id = resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
            let rollup_live_cursor =
                load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
            let hourly_rows = query_invocation_hourly_rollup_range_tx(
                tx.as_mut(),
                hourly_cursor,
                hourly_end_epoch,
                source_scope,
            )
            .await?;
            let mut exact_records = query_invocation_exact_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                snapshot_id,
            )
            .await?;
            let tail_records = query_invocation_full_hour_tail_records_tx(
                tx.as_mut(),
                &range_plan,
                source_scope,
                rollup_live_cursor,
                snapshot_id,
            )
            .await?;
            let archive_overlap_ids = tail_records
                .iter()
                .map(|record| record.id)
                .collect::<HashSet<_>>();
            exact_records.extend(tail_records);
            (snapshot_id, hourly_rows, exact_records, archive_overlap_ids)
        } else {
            let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
            let exact_records =
                query_invocation_exact_records(&state.pool, &range_plan, source_scope, snapshot_id)
                    .await?;
            (snapshot_id, Vec::new(), exact_records, HashSet::new())
        };
    let archived_hourly_rows = if let Some((range_start_epoch, range_end_epoch)) =
        range_plan.full_hour_range
    {
        let archived_start = Utc
            .timestamp_opt(range_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived timeseries start epoch")))?;
        let archived_end = Utc
            .timestamp_opt(range_end_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid archived timeseries end epoch")))?;
        crate::stats::query_unmaterialized_invocation_archive_hourly_rollup_deltas(
            &state.pool,
            source_scope,
            Some((archived_start, archived_end)),
            Some(&archive_overlap_ids),
        )
        .await?
    } else {
        Vec::new()
    };

    for row in hourly_rows.into_iter().chain(archived_hourly_rows) {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        let entry = aggregates.entry(bucket_epoch).or_default();
        entry.total_count += row.total_count;
        entry.success_count += row.success_count;
        entry.failure_count += row.failure_count;
        entry.total_tokens += row.total_tokens;
        entry.cache_input_tokens += row.cache_input_tokens;
        entry.total_cost += row.total_cost;
        entry.non_success_cost += row.non_success_cost;
        entry.total_latency_sample_count += row.total_latency_sample_count;
        entry.total_latency_sum_ms += row.total_latency_sum_ms;
        entry.first_byte_sample_count += row.first_byte_sample_count;
        entry.first_byte_ttfb_sum_ms += row.first_byte_sum_ms;
        entry.first_byte_histogram = if entry.first_byte_histogram.is_empty() {
            decode_approx_histogram(&row.first_byte_histogram)
        } else {
            let mut merged = entry.first_byte_histogram.clone();
            merge_approx_histogram_into(
                &mut merged,
                &decode_approx_histogram(&row.first_byte_histogram),
            )?;
            merged
        };
        entry.first_response_byte_total_sample_count += row.first_response_byte_total_sample_count;
        entry.first_response_byte_total_sum_ms += row.first_response_byte_total_sum_ms;
        entry.first_response_byte_total_histogram =
            if entry.first_response_byte_total_histogram.is_empty() {
                decode_approx_histogram(&row.first_response_byte_total_histogram)
            } else {
                let mut merged = entry.first_response_byte_total_histogram.clone();
                merge_approx_histogram_into(
                    &mut merged,
                    &decode_approx_histogram(&row.first_response_byte_total_histogram),
                )?;
                merged
            };
        entry.first_token_sample_count += row.first_token_sample_count;
        entry.first_token_sum_ms += row.first_token_sum_ms;
        entry.first_token_histogram = if entry.first_token_histogram.is_empty() {
            decode_approx_histogram(&row.first_token_histogram)
        } else {
            let mut merged = entry.first_token_histogram.clone();
            merge_approx_histogram_into(
                &mut merged,
                &decode_approx_histogram(&row.first_token_histogram),
            )?;
            merged
        };
    }
    let db_runtime_records = collect_in_flight_aggregate_records(&exact_records);
    for record in exact_records {
        let Some(occurred_utc) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let bucket_epoch =
            align_reporting_bucket_epoch(occurred_utc.timestamp(), bucket_seconds, reporting_tz)?;
        if let Some(entry) = aggregates.get_mut(&bucket_epoch) {
            entry.total_count += 1;
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            let is_success_like = prompt_shared::prompt_invocation_status_is_success_like(
                record.status.as_deref(),
                record.error_message.as_deref(),
            ) && classification.failure_class == FailureClass::None;
            if is_success_like {
                entry.success_count += 1;
            } else if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
                entry.in_flight_count += 1;
                entry
                    .in_flight_phase_counts
                    .increment_phase_name(record.live_phase.as_deref());
            } else if prompt_shared::prompt_invocation_status_counts_toward_terminal_totals(
                record.status.as_deref(),
            ) && classification.failure_class != FailureClass::None
            {
                entry.failure_count += 1;
            }
            let latency_status = if is_success_like {
                Some("success")
            } else {
                record.status.as_deref()
            };
            entry.record_exact_ttfb_sample(latency_status, record.t_upstream_ttfb_ms);
            entry.record_exact_first_response_byte_total_sample(
                record.t_req_read_ms,
                record.t_req_parse_ms,
                record.t_upstream_connect_ms,
                record.t_upstream_ttfb_ms,
            );
            entry.record_first_token_sample(record.first_token_ms);
            entry.total_tokens += record.total_tokens.unwrap_or_default();
            entry.cache_input_tokens += record.cache_input_tokens.unwrap_or_default();
            let cost = record.cost.unwrap_or_default();
            entry.total_cost += cost;
            if invocation_counts_toward_non_success_usage(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            ) {
                entry.non_success_cost += cost;
            }
        }
    }

    overlay_runtime_timeseries_in_flight(
        state.as_ref(),
        &mut aggregates,
        source_scope,
        None,
        range_window.start,
        range_window.end,
        bucket_seconds,
        reporting_tz,
        &db_runtime_records,
    )?;

    let mut points = Vec::with_capacity(aggregates.len());
    for (bucket_epoch, agg) in aggregates {
        let bucket_end_epoch =
            next_reporting_bucket_epoch(bucket_epoch, bucket_seconds, reporting_tz)?;
        if bucket_epoch < fill_start_epoch || bucket_end_epoch > fill_end_epoch {
            continue;
        }
        let start = Utc
            .timestamp_opt(bucket_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        let end = Utc
            .timestamp_opt(bucket_end_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
        points.push(timeseries_point_from_aggregate(start, end, &agg));
    }

    Ok(Json(TimeseriesResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        bucket_seconds,
        snapshot_id,
        effective_bucket: bucket_selection.effective_bucket,
        available_buckets: bucket_selection.available_buckets,
        bucket_limited_to_daily: bucket_selection.bucket_limited_to_daily,
        points,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeseries_point_clears_latency_when_bucket_has_no_calls() {
        let mut aggregate = BucketAggregate {
            first_byte_sample_count: 1,
            first_byte_ttfb_sum_ms: 750.0,
            first_response_byte_total_sample_count: 1,
            first_response_byte_total_sum_ms: 18_225.02,
            total_latency_sample_count: 1,
            total_latency_sum_ms: 24_000.0,
            ..Default::default()
        };
        aggregate.first_byte_ttfb_values.push(750.0);
        aggregate.first_response_byte_total_values.push(18_225.02);

        let point = timeseries_point_from_aggregate(
            Utc.timestamp_opt(1_775_608_200, 0)
                .single()
                .expect("valid start timestamp"),
            Utc.timestamp_opt(1_775_608_260, 0)
                .single()
                .expect("valid end timestamp"),
            &aggregate,
        );

        assert_eq!(point.total_count, 0);
        assert_eq!(point.first_byte_sample_count, 0);
        assert!(point.first_byte_avg_ms.is_none());
        assert!(point.first_byte_p95_ms.is_none());
        assert_eq!(point.first_response_byte_total_sample_count, 0);
        assert!(point.first_response_byte_total_avg_ms.is_none());
        assert!(point.first_response_byte_total_p95_ms.is_none());
        assert!(point.avg_total_ms.is_none());
    }

    #[test]
    fn timeseries_point_keeps_rollup_backed_total_latency_average() {
        let aggregate = BucketAggregate {
            total_count: 4,
            success_count: 3,
            failure_count: 1,
            total_latency_sample_count: 2,
            total_latency_sum_ms: 1_800.0,
            ..Default::default()
        };

        let point = timeseries_point_from_aggregate(
            Utc.timestamp_opt(1_775_608_200, 0)
                .single()
                .expect("valid start timestamp"),
            Utc.timestamp_opt(1_775_608_260, 0)
                .single()
                .expect("valid end timestamp"),
            &aggregate,
        );

        assert_eq!(point.total_latency_sample_count, 2);
        assert_eq!(point.avg_total_ms, Some(900.0));
    }

    #[test]
    fn timeseries_point_exports_in_flight_phase_counts_and_compat_total() {
        let mut aggregate = BucketAggregate {
            total_count: 5,
            success_count: 1,
            failure_count: 1,
            in_flight_count: 3,
            ..Default::default()
        };
        aggregate.in_flight_phase_counts.queued = 1;
        aggregate.in_flight_phase_counts.requesting = 1;
        aggregate.in_flight_phase_counts.responding = 1;

        let point = timeseries_point_from_aggregate(
            Utc.timestamp_opt(1_775_608_200, 0)
                .single()
                .expect("valid start timestamp"),
            Utc.timestamp_opt(1_775_608_260, 0)
                .single()
                .expect("valid end timestamp"),
            &aggregate,
        );

        assert_eq!(point.in_flight_count, 3);
        assert_eq!(point.in_flight_phase_counts.queued, 1);
        assert_eq!(point.in_flight_phase_counts.requesting, 1);
        assert_eq!(point.in_flight_phase_counts.responding, 1);
    }

    #[test]
    fn add_rollup_rows_preserves_total_latency_metrics() {
        let bucket_epoch =
            align_reporting_bucket_epoch(1_775_608_200, 3_600, chrono_tz::Asia::Shanghai)
                .expect("aligned bucket");
        let mut aggregates = BTreeMap::from([(bucket_epoch, BucketAggregate::default())]);
        add_rollup_rows_to_timeseries_aggregates(
            &mut aggregates,
            vec![UpstreamAccountStatsRollupRecord {
                bucket_start_epoch: 1_775_608_200,
                total_count: 3,
                success_count: 2,
                failure_count: 1,
                in_flight_count: 0,
                total_tokens: 99,
                input_tokens: 0,
                output_tokens: 0,
                cache_input_tokens: 7,
                total_cost: 1.25,
                non_success_cost: 0.3,
                total_latency_sample_count: 2,
                total_latency_sum_ms: 1_100.0,
                first_byte_sample_count: 1,
                first_byte_sum_ms: 450.0,
                first_byte_max_ms: 450.0,
                first_byte_histogram: encode_approx_histogram(&[0, 1]).expect("histogram"),
                first_response_byte_total_sample_count: 1,
                first_response_byte_total_sum_ms: 900.0,
                first_response_byte_total_max_ms: 900.0,
                first_response_byte_total_histogram: encode_approx_histogram(&[0, 1])
                    .expect("histogram"),
                first_token_sample_count: 0,
                first_token_sum_ms: 0.0,
                first_token_max_ms: 0.0,
                first_token_histogram: encode_approx_histogram(&[]).expect("histogram"),
            }],
            3_600,
            chrono_tz::Asia::Shanghai,
        )
        .expect("rollup rows should aggregate");

        let aggregate = aggregates.get(&bucket_epoch).expect("bucket");
        assert_eq!(aggregate.total_count, 3);
        assert_eq!(aggregate.total_latency_sample_count, 2);
        assert_eq!(aggregate.total_latency_sum_ms, 1_100.0);
    }
}
