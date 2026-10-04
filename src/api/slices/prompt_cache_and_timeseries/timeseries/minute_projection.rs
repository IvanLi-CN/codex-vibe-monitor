use super::aggregation::{
    add_exact_record_to_timeseries_aggregate, add_timeseries_terminal_delta_to_aggregate,
};
use super::prompt_cache_and_timeseries_shared as prompt_shared;
use super::*;
#[path = "recovery.rs"]
mod recovery;
pub(crate) use recovery::{
    prepare_timeseries_minute_projection_after_restart,
    prepare_timeseries_minute_projection_after_restart_for_managed_run,
};
#[cfg(test)]
#[derive(sqlx::FromRow)]
pub(crate) struct TimeseriesMinuteProjectionRow {
    minute_start_epoch: i64,
    records_json: String,
    max_row_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct TimeseriesMinuteProjectionV2Row {
    minute_start_epoch: i64,
    aggregate_json: String,
    total_latency_samples_json: String,
    first_byte_samples_json: String,
    first_response_byte_total_samples_json: String,
    first_token_samples_json: String,
    max_row_id: i64,
    coverage_state: String,
}

pub(crate) struct TimeseriesMinuteProjectionV2Load {
    pub(crate) aggregates: BTreeMap<i64, BucketAggregate>,
    pub(crate) cursor: i64,
    pub(crate) snapshot_fence: TimeseriesMinuteProjectionSnapshotFence,
}

#[derive(Debug, Clone)]
pub(crate) struct TimeseriesMinuteProjectionSnapshotFence {
    rows: Vec<TimeseriesMinuteProjectionV2Row>,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TimeseriesMinuteProjectionEligibility {
    candidate: bool,
    durable_replacement_fence_installed: bool,
    durable_recovery_pending: bool,
    uncovered_terminal_delta: bool,
    coverage_invalidation_pending: bool,
    can_warm: bool,
}

impl TimeseriesMinuteProjectionEligibility {
    pub(crate) fn is_candidate(self) -> bool {
        self.candidate
    }

    pub(crate) fn can_load(self) -> bool {
        self.candidate
            && self.durable_replacement_fence_installed
            && !self.coverage_invalidation_pending
    }

    pub(crate) fn can_warm(self) -> bool {
        self.can_warm
    }

    pub(crate) fn coverage_invalidation_pending(self) -> bool {
        self.coverage_invalidation_pending
    }

    pub(crate) fn has_uncovered_terminal_delta(self) -> bool {
        self.uncovered_terminal_delta
    }

    pub(crate) fn mark_coverage_invalidated(&mut self) {
        self.coverage_invalidation_pending = true;
    }
}

pub(crate) const TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT: usize = 64;
pub(crate) const TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT: usize = 512;
pub(crate) const TIMESERIES_MINUTE_PROJECTION_INVALIDATION_ROW_BATCH_LIMIT: i64 = 256;
pub(crate) const TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER: &str = "timeseries_minute_v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeseriesMinuteProjectionFlushOutcome {
    Flushed,
    Deferred(TimeseriesMinuteProjectionDeferred),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimeseriesMinuteProjectionDeferred {
    pub(crate) retry_after: Option<Duration>,
}

pub(crate) enum TimeseriesMinuteProjectionWriteAdmissionOutcome {
    Acquired(TimeseriesMinuteProjectionWriteAdmission),
    Deferred(TimeseriesMinuteProjectionDeferred),
}

pub(crate) struct TimeseriesMinuteProjectionWriteAdmission {
    _write_permit: crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
    _pressure_permit: crate::db_pressure::DbBackgroundPermit,
}

#[derive(Debug, Default)]
pub(crate) struct TimeseriesMinuteProjectionCoverageInvalidationStats {
    row_count: u64,
    transaction_count: u64,
}

pub(crate) enum TimeseriesMinuteProjectionCoverageInvalidationOutcome {
    Invalidated(TimeseriesMinuteProjectionCoverageInvalidationStats),
    Deferred(TimeseriesMinuteProjectionDeferred),
    Cancelled,
}

pub(crate) fn timeseries_minute_projection_is_cancelled(
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> bool {
    cancellation.is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
}

pub(crate) fn timeseries_minute_projection_pressure_deferred(
    pressure_gate: &crate::db_pressure::DbPressureGate,
    task: &'static str,
    error: &ApiError,
) -> Option<TimeseriesMinuteProjectionDeferred> {
    let error = match error {
        ApiError::BadRequest(error)
        | ApiError::Conflict(error)
        | ApiError::Unavailable(error)
        | ApiError::Internal(error) => error,
    };
    if !pressure_gate.record_error(task, error) {
        return None;
    }
    let retry_after = Duration::from_millis(
        pressure_gate
            .snapshot()
            .pressure_cooldown_remaining_ms
            .max(1),
    );
    Some(TimeseriesMinuteProjectionDeferred {
        retry_after: Some(retry_after),
    })
}

pub(crate) fn timeseries_projection_scope(source_scope: InvocationSourceScope) -> &'static str {
    match source_scope {
        InvocationSourceScope::All => "all",
        InvocationSourceScope::ProxyOnly => "proxy_only",
    }
}

pub(crate) fn timeseries_projection_snapshot_records(
    source_records: &[InvocationAggregateRecord],
    pending_terminal_deltas: &[(u64, i64, TimeseriesTerminalDelta)],
) -> Vec<TimeseriesProjectionSnapshotRecord> {
    let terminal_source_rows = source_records
        .iter()
        .filter(|record| !prompt_shared::invocation_status_is_in_flight(record.status.as_deref()))
        .map(|record| (record.id, (&record.invoke_id, &record.occurred_at)))
        .collect::<HashMap<_, _>>();

    pending_terminal_deltas
        .iter()
        .filter_map(|(_, row_id, delta)| {
            terminal_source_rows
                .get(row_id)
                .map(
                    |(invoke_id, occurred_at)| TimeseriesProjectionSnapshotRecord {
                        row_id: *row_id,
                        invoke_id: (*invoke_id).clone(),
                        occurred_at: (*occurred_at).clone(),
                        delta: delta.clone(),
                    },
                )
        })
        .collect()
}

pub(crate) fn timeseries_minute_projection_has_uncovered_terminal_delta(
    terminal_projection_hub: &TerminalProjectionHub,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> bool {
    let (full_minute_start_epoch, full_minute_end_epoch) = complete_minute_bounds(start, end);
    if full_minute_start_epoch >= full_minute_end_epoch {
        return false;
    }
    let selection = TimeseriesProjectionSelection {
        source_scope: timeseries_projection_scope(source_scope),
        upstream_account_id,
    };
    // A cursor does not version a row replacement. A persisted terminal delta that has not been
    // committed into this selection's warm snapshot makes its covered minute unsafe to reuse.
    terminal_projection_hub
        .pending_timeseries_deltas_for_selection(selection, 10_000)
        .into_iter()
        .any(|(_, _, delta)| {
            parse_to_utc_datetime(&delta.occurred_at).is_none_or(|occurred| {
                let occurred_epoch = occurred.timestamp();
                occurred_epoch >= full_minute_start_epoch && occurred_epoch < full_minute_end_epoch
            })
        })
}

pub(crate) async fn timeseries_minute_projection_non_proxy_terminal_replacement_fence_is_installed(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<bool, ApiError> {
    if source_scope == InvocationSourceScope::ProxyOnly {
        return Ok(true);
    }

    let installed = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'trg_timeseries_minute_projection_non_proxy_terminal_replacement')",
    )
    .fetch_one(pool)
    .await?;
    Ok(installed != 0)
}

pub(crate) async fn mark_timeseries_minute_projection_startup_recovery(
    pool: &Pool<Sqlite>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2_recovery (consumer, generation, invalidation_pending, updated_at) VALUES (?1, 1, 1, datetime('now')) ON CONFLICT(consumer) DO UPDATE SET generation = timeseries_minute_projection_v2_recovery.generation + 1, invalidation_pending = 1, updated_at = excluded.updated_at",
    )
    .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn timeseries_minute_projection_recovery_generation(
    pool: &Pool<Sqlite>,
) -> Result<Option<i64>, ApiError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT generation FROM timeseries_minute_projection_v2_recovery WHERE consumer = ?1 AND invalidation_pending = 1",
    )
    .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
    .fetch_optional(pool)
    .await
    .map_err(ApiError::from)
}

pub(crate) async fn timeseries_minute_projection_recovery_pending(
    pool: &Pool<Sqlite>,
) -> Result<bool, ApiError> {
    Ok(timeseries_minute_projection_recovery_generation(pool)
        .await?
        .is_some())
}

pub(crate) async fn timeseries_minute_projection_v2_has_warming_coverage(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<bool, ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    if minute_end <= minute_start {
        return Ok(false);
    }

    let warming_coverage_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE minute_start_epoch >= ?1 AND minute_start_epoch < ?2 AND source_scope = ?3 AND upstream_account_key = ?4 AND coverage_state <> 'ready'",
    )
    .bind(minute_start)
    .bind(minute_end)
    .bind(timeseries_projection_scope(source_scope))
    .bind(upstream_account_id.unwrap_or(-1))
    .fetch_one(pool)
    .await?;
    Ok(warming_coverage_count != 0)
}

pub(crate) async fn evaluate_timeseries_minute_projection_eligibility(
    pool: &Pool<Sqlite>,
    terminal_projection_hub: &TerminalProjectionHub,
    candidate: bool,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<TimeseriesMinuteProjectionEligibility, ApiError> {
    if !candidate {
        return Ok(TimeseriesMinuteProjectionEligibility::default());
    }

    let durable_replacement_fence_installed =
        timeseries_minute_projection_non_proxy_terminal_replacement_fence_is_installed(
            pool,
            source_scope,
        )
        .await?;
    let durable_recovery_pending = timeseries_minute_projection_recovery_pending(pool).await?;
    let uncovered_terminal_delta = timeseries_minute_projection_has_uncovered_terminal_delta(
        terminal_projection_hub,
        source_scope,
        upstream_account_id,
        start,
        end,
    );
    let warming_projection_coverage = timeseries_minute_projection_v2_has_warming_coverage(
        pool,
        start,
        end,
        source_scope,
        upstream_account_id,
    )
    .await?;
    let coverage_invalidation_pending = terminal_projection_hub
        .timeseries_coverage_invalidation_pending()
        .is_some()
        || uncovered_terminal_delta
        || warming_projection_coverage
        || durable_recovery_pending
        || !durable_replacement_fence_installed;
    let can_warm = durable_replacement_fence_installed
        && !durable_recovery_pending
        && terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_none()
        && !uncovered_terminal_delta;

    Ok(TimeseriesMinuteProjectionEligibility {
        candidate,
        durable_replacement_fence_installed,
        durable_recovery_pending,
        uncovered_terminal_delta,
        coverage_invalidation_pending,
        can_warm,
    })
}

pub(crate) fn complete_minute_bounds(start: DateTime<Utc>, end: DateTime<Utc>) -> (i64, i64) {
    let start_epoch = (start.timestamp() + 59).div_euclid(60) * 60;
    let end_epoch = end.timestamp().div_euclid(60) * 60;
    (start_epoch, end_epoch.max(start_epoch))
}

#[cfg(test)]
pub(crate) async fn load_timeseries_minute_projection_records(
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
pub(crate) async fn store_timeseries_minute_projection_records(
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

pub(crate) async fn load_timeseries_minute_projection_v2(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<Option<TimeseriesMinuteProjectionV2Load>, ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    let Some(rows) = load_ready_timeseries_minute_projection_v2_rows(
        pool,
        start,
        end,
        source_scope,
        upstream_account_id,
    )
    .await?
    else {
        return Ok(None);
    };
    let mut aggregates = BTreeMap::new();
    let mut cursor = 0;
    for row in &rows {
        debug_assert_eq!(row.coverage_state, "ready");
        let mut aggregate = serde_json::from_str::<BucketAggregate>(&row.aggregate_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 minute aggregate: {err}")))?;
        // Keep the sample payloads independently addressable so a corrupt aggregate payload
        // cannot silently turn an exact P95 into a histogram approximation.
        aggregate.total_latency_values = serde_json::from_str(&row.total_latency_samples_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 total-latency samples: {err}")))?;
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
    Ok(Some(TimeseriesMinuteProjectionV2Load {
        aggregates,
        cursor,
        snapshot_fence: TimeseriesMinuteProjectionSnapshotFence {
            rows,
            start_epoch: minute_start,
            end_epoch: minute_end,
            source_scope,
            upstream_account_id,
        },
    }))
}

pub(crate) async fn load_ready_timeseries_minute_projection_v2_rows(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<Option<Vec<TimeseriesMinuteProjectionV2Row>>, ApiError> {
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
    Ok(Some(rows))
}

pub(crate) async fn timeseries_minute_projection_v2_snapshot_is_current(
    pool: &Pool<Sqlite>,
    terminal_projection_hub: &TerminalProjectionHub,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    snapshot_fence: &TimeseriesMinuteProjectionSnapshotFence,
) -> Result<bool, ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    if snapshot_fence.start_epoch != minute_start
        || snapshot_fence.end_epoch != minute_end
        || snapshot_fence.source_scope != source_scope
        || snapshot_fence.upstream_account_id != upstream_account_id
    {
        return Ok(false);
    }
    if timeseries_minute_projection_has_uncovered_terminal_delta(
        terminal_projection_hub,
        source_scope,
        upstream_account_id,
        start,
        end,
    ) || terminal_projection_hub
        .timeseries_coverage_invalidation_pending()
        .is_some()
        || timeseries_minute_projection_recovery_pending(pool).await?
    {
        return Ok(false);
    }
    // Re-read the payload selected by the loader, not merely its row count. An invalidation can
    // warm the same primary keys again while a reader is building its live tail.
    let projection_rows_are_current = load_ready_timeseries_minute_projection_v2_rows(
        pool,
        start,
        end,
        source_scope,
        upstream_account_id,
    )
    .await?
    .is_some_and(|current_rows| current_rows == snapshot_fence.rows);
    Ok(projection_rows_are_current
        && !timeseries_minute_projection_has_uncovered_terminal_delta(
            terminal_projection_hub,
            source_scope,
            upstream_account_id,
            start,
            end,
        )
        && terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_none()
        && !timeseries_minute_projection_recovery_pending(pool).await?)
}

#[cfg(test)]
pub(crate) async fn store_timeseries_minute_projection_v2_for_test(
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
        let total_latency_samples_json =
            serde_json::to_string(&aggregate.total_latency_values).map_err(ApiError::from)?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeseriesMinuteProjectionWarmOutcome {
    Stored,
    Deferred(TimeseriesMinuteProjectionDeferred),
}

pub(crate) async fn store_timeseries_minute_projection_v2_warm(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &TerminalProjectionHub,
    trigger: &'static str,
) -> Result<TimeseriesMinuteProjectionWarmOutcome, ApiError> {
    let outcome = store_timeseries_minute_projection_v2_warm_with_coordinator(
        pool,
        start,
        end,
        source_scope,
        upstream_account_id,
        terminal_projection_hub,
        trigger,
        &crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
    )
    .await;
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            let gate = crate::db_pressure::global_db_pressure_gate();
            if let Some(deferred) = timeseries_minute_projection_pressure_deferred(
                gate,
                "timeseries_minute_projection_warm",
                &error,
            ) {
                Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(deferred))
            } else {
                Err(error)
            }
        }
    }
}

pub(crate) async fn store_timeseries_minute_projection_v2_warm_with_eligibility_retry(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &TerminalProjectionHub,
    trigger: &'static str,
) -> Result<TimeseriesMinuteProjectionWarmOutcome, ApiError> {
    let pressure_gate = crate::db_pressure::global_db_pressure_gate();
    let outcome = store_timeseries_minute_projection_v2_warm(
        pool,
        start,
        end,
        source_scope,
        upstream_account_id,
        terminal_projection_hub,
        trigger,
    )
    .await?;
    let TimeseriesMinuteProjectionWarmOutcome::Deferred(deferred) = outcome else {
        return Ok(outcome);
    };
    let observed_eligibility_generation = pressure_gate.eligibility_generation();
    debug!(
        route = "timeseries_projection",
        builder = "minute_projection_v2",
        trigger,
        transaction_phase = "exact_warm",
        observed_eligibility_generation,
        "waiting briefly to retry a deferred exact-fallback minute projection warm write"
    );
    tokio::select! {
        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {
            store_timeseries_minute_projection_v2_warm(
                pool,
                start,
                end,
                source_scope,
                upstream_account_id,
                terminal_projection_hub,
                trigger,
            ).await
        }
        _ = tokio::time::sleep(deferred.retry_after.unwrap_or(Duration::from_secs(5))) => {
            store_timeseries_minute_projection_v2_warm(
                pool,
                start,
                end,
                source_scope,
                upstream_account_id,
                terminal_projection_hub,
                trigger,
            ).await
        }
    }
}

pub(crate) async fn store_timeseries_minute_projection_v2_warm_with_coordinator(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &TerminalProjectionHub,
    trigger: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Result<TimeseriesMinuteProjectionWarmOutcome, ApiError> {
    let (minute_start, minute_end) = complete_minute_bounds(start, end);
    if minute_end <= minute_start {
        return Ok(TimeseriesMinuteProjectionWarmOutcome::Stored);
    }
    let keys = (minute_start..minute_end)
        .step_by(60_usize)
        .map(|minute| TimeseriesMinuteProjectionKey {
            minute_start_epoch: minute,
            source_scope: timeseries_projection_scope(source_scope),
            upstream_account_key: upstream_account_id.unwrap_or(-1),
        })
        .collect::<Vec<_>>();
    let projection_selection = TimeseriesProjectionSelection {
        source_scope: timeseries_projection_scope(source_scope),
        upstream_account_id,
    };

    for key_batch in keys.chunks(TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT) {
        if timeseries_minute_projection_recovery_pending(pool).await? {
            debug!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                trigger,
                transaction_phase = "exact_warm",
                admission_outcome = "deferred",
                defer_reason = "durable_recovery_pending",
                "minute projection warm write deferred while durable recovery is pending"
            );
            return Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after: None },
            ));
        }
        if terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_some()
        {
            debug!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                trigger,
                transaction_phase = "exact_warm",
                admission_outcome = "deferred",
                defer_reason = "coverage_invalidation_pending",
                "minute projection warm write deferred while coverage invalidation is pending"
            );
            return Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after: None },
            ));
        }
        let batch_start = Utc
            .timestamp_opt(key_batch[0].minute_start_epoch, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid minute projection batch start")))?;
        let batch_end = Utc
            .timestamp_opt(key_batch[key_batch.len() - 1].minute_start_epoch + 60, 0)
            .single()
            .ok_or_else(|| ApiError::from(anyhow!("invalid minute projection batch end")))?;
        let range = ExactUtcRange {
            start: batch_start,
            end: batch_end,
        };
        // Source scans and aggregate serialization deliberately happen before P2 admission. A
        // hot minute can contain an unbounded number of durable rows, but only the prepared row
        // writes may hold the P2 permit or SQLite's immediate writer barrier.
        let source_snapshot_id = resolve_invocation_snapshot_id(pool, source_scope).await?;
        let source_records = match upstream_account_id {
            Some(upstream_account_id) => {
                query_invocation_aggregate_records_from_live_range_for_account(
                    pool,
                    range,
                    source_scope,
                    None,
                    Some(source_snapshot_id),
                    upstream_account_id,
                )
                .await?
            }
            None => {
                query_invocation_aggregate_records_from_live_range(
                    pool,
                    range,
                    source_scope,
                    None,
                    Some(source_snapshot_id),
                )
                .await?
            }
        };
        let pending_terminal_deltas = terminal_projection_hub
            .pending_timeseries_deltas_for_selection(projection_selection, 10_000);
        let projection_snapshot_records =
            timeseries_projection_snapshot_records(&source_records, &pending_terminal_deltas);
        let mut aggregates = HashMap::<i64, BucketAggregate>::new();
        let mut max_row_ids = HashMap::<i64, i64>::new();
        for record in source_records {
            if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
                continue;
            }
            let Some(occurred) = parse_to_utc_datetime(&record.occurred_at) else {
                continue;
            };
            let minute = occurred.timestamp().div_euclid(60) * 60;
            add_exact_record_to_timeseries_aggregate(
                aggregates.entry(minute).or_default(),
                &record,
            );
            max_row_ids
                .entry(minute)
                .and_modify(|max_row_id| *max_row_id = (*max_row_id).max(record.id))
                .or_insert(record.id);
        }
        let mut prepared_writes = Vec::with_capacity(key_batch.len());
        for key in key_batch {
            let aggregate = aggregates
                .remove(&key.minute_start_epoch)
                .unwrap_or_default();
            let max_row_id = max_row_ids.remove(&key.minute_start_epoch).unwrap_or(0);
            prepared_writes.push(prepare_timeseries_minute_projection_v2_write(
                key, &aggregate, max_row_id,
            )?);
        }
        let admission = match try_acquire_timeseries_minute_projection_write(
            coordinator,
            crate::db_pressure::global_db_pressure_gate(),
            trigger,
            "exact_warm",
            key_batch.len(),
        )
        .await
        {
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
                return Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(deferred));
            }
        };
        if terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_some()
        {
            drop(admission);
            return Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after: None },
            ));
        }
        let transaction_started = Instant::now();
        // The writer barrier only protects the prepared upserts. A direct terminal update can
        // invalidate this coverage through SQLite triggers without reaching the proxy hub, so
        // recheck its durable marker and the source snapshot after acquiring the barrier.
        let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
        let durable_recovery_pending = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM timeseries_minute_projection_v2_recovery WHERE consumer = ?1 AND invalidation_pending = 1)",
        )
        .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
        .fetch_one(tx.as_mut())
        .await?
            != 0;
        if durable_recovery_pending {
            return Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after: None },
            ));
        }
        if resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await? != source_snapshot_id
        {
            return Ok(TimeseriesMinuteProjectionWarmOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after: None },
            ));
        }
        for write in &prepared_writes {
            upsert_timeseries_minute_projection_v2_prepared_write_tx(tx.as_mut(), write).await?;
        }
        tx.commit().await?;
        let coverage_ack_count = terminal_projection_hub
            .mark_timeseries_warm_coverage(projection_selection, &projection_snapshot_records);
        debug!(
            route = "timeseries_projection",
            builder = "minute_projection_v2",
            trigger,
            transaction_phase = "exact_warm",
            transaction_key_limit = TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT,
            transaction_key_count = key_batch.len(),
            coverage_ack_count,
            elapsed_ms = transaction_started.elapsed().as_millis() as u64,
            "materialized and acknowledged a bounded exact-fallback minute projection slice"
        );
        drop(admission);
        tokio::task::yield_now().await;
    }
    Ok(TimeseriesMinuteProjectionWarmOutcome::Stored)
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) struct TimeseriesMinuteProjectionKey {
    minute_start_epoch: i64,
    source_scope: &'static str,
    upstream_account_key: i64,
}

pub(crate) struct TimeseriesMinuteProjectionPreparedWrite {
    key: TimeseriesMinuteProjectionKey,
    aggregate_json: String,
    total_latency_samples_json: String,
    first_byte_samples_json: String,
    first_response_byte_total_samples_json: String,
    first_token_samples_json: String,
    max_row_id: i64,
}

pub(crate) fn add_timeseries_delta_projection_keys(
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

pub(crate) async fn load_timeseries_minute_projection_v2_key(
    pool: &Pool<Sqlite>,
    key: &TimeseriesMinuteProjectionKey,
) -> Result<Option<(BucketAggregate, i64)>, ApiError> {
    let row = sqlx::query_as::<_, TimeseriesMinuteProjectionV2Row>(
        "SELECT minute_start_epoch, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state FROM timeseries_minute_projection_v2 WHERE minute_start_epoch = ?1 AND source_scope = ?2 AND upstream_account_key = ?3",
    )
    .bind(key.minute_start_epoch)
    .bind(key.source_scope)
    .bind(key.upstream_account_key)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    if row.coverage_state != "ready" {
        return Ok(None);
    }
    let mut aggregate = serde_json::from_str::<BucketAggregate>(&row.aggregate_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 minute aggregate: {err}")))?;
    aggregate.total_latency_values = serde_json::from_str(&row.total_latency_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 total-latency samples: {err}")))?;
    aggregate.first_byte_ttfb_values = serde_json::from_str(&row.first_byte_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 first-byte samples: {err}")))?;
    aggregate.first_response_byte_total_values =
        serde_json::from_str(&row.first_response_byte_total_samples_json)
            .map_err(|err| ApiError::from(anyhow!("invalid v2 first-response samples: {err}")))?;
    aggregate.first_token_values = serde_json::from_str(&row.first_token_samples_json)
        .map_err(|err| ApiError::from(anyhow!("invalid v2 first-token samples: {err}")))?;
    Ok(Some((aggregate, row.max_row_id)))
}

pub(crate) async fn rebuild_timeseries_minute_projection_v2_key(
    pool: &Pool<Sqlite>,
    key: &TimeseriesMinuteProjectionKey,
    snapshot_id: Option<i64>,
) -> Result<(BucketAggregate, i64, u64), ApiError> {
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
        query_invocation_aggregate_records_from_live_range(
            pool,
            range,
            source_scope,
            None,
            snapshot_id,
        )
        .await?
    } else {
        query_invocation_aggregate_records_from_live_range_for_account(
            pool,
            range,
            source_scope,
            None,
            snapshot_id,
            key.upstream_account_key,
        )
        .await?
    };
    let mut aggregate = BucketAggregate::default();
    let mut max_row_id = 0;
    let source_row_count = records.len() as u64;
    for record in records {
        if prompt_shared::invocation_status_is_in_flight(record.status.as_deref()) {
            continue;
        }
        max_row_id = max_row_id.max(record.id);
        add_exact_record_to_timeseries_aggregate(&mut aggregate, &record);
    }
    Ok((aggregate, max_row_id, source_row_count))
}

pub(crate) fn prepare_timeseries_minute_projection_v2_write(
    key: &TimeseriesMinuteProjectionKey,
    aggregate: &BucketAggregate,
    max_row_id: i64,
) -> Result<TimeseriesMinuteProjectionPreparedWrite, ApiError> {
    Ok(TimeseriesMinuteProjectionPreparedWrite {
        key: key.clone(),
        aggregate_json: serde_json::to_string(aggregate).map_err(ApiError::from)?,
        total_latency_samples_json: serde_json::to_string(&aggregate.total_latency_values)
            .map_err(ApiError::from)?,
        first_byte_samples_json: serde_json::to_string(&aggregate.first_byte_ttfb_values)
            .map_err(ApiError::from)?,
        first_response_byte_total_samples_json: serde_json::to_string(
            &aggregate.first_response_byte_total_values,
        )
        .map_err(ApiError::from)?,
        first_token_samples_json: serde_json::to_string(&aggregate.first_token_values)
            .map_err(ApiError::from)?,
        max_row_id,
    })
}

pub(crate) async fn upsert_timeseries_minute_projection_v2_prepared_write_tx(
    tx: &mut SqliteConnection,
    write: &TimeseriesMinuteProjectionPreparedWrite,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'ready', datetime('now')) ON CONFLICT(minute_start_epoch, source_scope, upstream_account_key) DO UPDATE SET aggregate_json = excluded.aggregate_json, total_latency_samples_json = excluded.total_latency_samples_json, first_byte_samples_json = excluded.first_byte_samples_json, first_response_byte_total_samples_json = excluded.first_response_byte_total_samples_json, first_token_samples_json = excluded.first_token_samples_json, max_row_id = MAX(timeseries_minute_projection_v2.max_row_id, excluded.max_row_id), coverage_state = 'ready', updated_at = excluded.updated_at",
    )
    .bind(write.key.minute_start_epoch)
    .bind(write.key.source_scope)
    .bind(write.key.upstream_account_key)
    .bind(&write.aggregate_json)
    .bind(&write.total_latency_samples_json)
    .bind(&write.first_byte_samples_json)
    .bind(&write.first_response_byte_total_samples_json)
    .bind(&write.first_token_samples_json)
    .bind(write.max_row_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub(crate) fn timeseries_projection_requires_exact_rebuild(
    deltas: &[(i64, TimeseriesTerminalDelta)],
    existing_max_row_id: i64,
) -> bool {
    let mut seen_row_ids = HashSet::with_capacity(deltas.len());
    deltas
        .iter()
        .any(|(row_id, _)| *row_id <= existing_max_row_id || !seen_row_ids.insert(*row_id))
}

pub(crate) async fn try_acquire_timeseries_minute_projection_write(
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    pressure_gate: &crate::db_pressure::DbPressureGate,
    trigger: &'static str,
    transaction_phase: &'static str,
    pending_event_count: usize,
) -> TimeseriesMinuteProjectionWriteAdmissionOutcome {
    let observed_eligibility_generation = pressure_gate.eligibility_generation();
    let Some(mut write_permit) = coordinator
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
    else {
        let snapshot = coordinator.snapshot().await;
        debug!(
            route = "timeseries_projection",
            builder = "minute_projection_v2",
            trigger,
            transaction_phase,
            admission_outcome = "deferred",
            defer_reason = "coordinator_priority",
            active_write_class = snapshot.active_write_class.as_deref().unwrap_or("none"),
            p1_waiter_count = snapshot.p1_waiter_count,
            interactive_waiter_count = snapshot.interactive_waiter_count,
            p2_waiter_count = snapshot.p2_waiter_count,
            pending_event_count,
            "minute projection deferred before opening a P2 write transaction"
        );
        return TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred { retry_after: None },
        );
    };

    match pressure_gate.try_begin_background("timeseries_minute_projection_flush") {
        Ok(pressure_permit) => TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(
            TimeseriesMinuteProjectionWriteAdmission {
                _write_permit: write_permit,
                _pressure_permit: pressure_permit,
            },
        ),
        Err(reason) => {
            let retry_after = match reason {
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms } => {
                    Some(Duration::from_millis(remaining_ms.max(1)))
                }
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy => None,
            };
            // This acquisition never opened a background transaction. Its P2 permit must not
            // satisfy the retry wait that was registered before this admission attempt.
            write_permit.suppress_background_eligibility_wakeup();
            drop(write_permit);
            debug!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                trigger,
                transaction_phase,
                admission_outcome = "deferred",
                defer_reason = "writer_pressure",
                observed_eligibility_generation,
                pending_event_count,
                %reason,
                "minute projection deferred before opening a P2 write transaction"
            );
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(
                TimeseriesMinuteProjectionDeferred { retry_after },
            )
        }
    }
}

pub(crate) fn begin_timeseries_minute_projection_observation(
    trigger: &'static str,
    managed_run_id: Option<i64>,
) -> crate::TaskExecutionObservation {
    managed_run_id
        .and_then(crate::TaskExecutionObservation::for_managed_run)
        .unwrap_or_else(|| {
            crate::TaskExecutionObservation::begin(
                "timeseries_minute_projection",
                &crate::maintenance_store::task_title_for_observation(
                    "timeseries_minute_projection",
                ),
                trigger,
                crate::maintenance_store::task_execution_class("timeseries_minute_projection"),
                "processing",
            )
        })
}

pub(crate) async fn invalidate_timeseries_minute_projection_coverage(
    state: &AppState,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    trigger: &'static str,
    pending_event_count: usize,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
    managed_run_id: Option<i64>,
    observation: &mut Option<crate::TaskExecutionObservation>,
) -> Result<TimeseriesMinuteProjectionCoverageInvalidationOutcome, ApiError> {
    let mut stats = TimeseriesMinuteProjectionCoverageInvalidationStats::default();
    loop {
        if timeseries_minute_projection_is_cancelled(cancellation) {
            return Ok(TimeseriesMinuteProjectionCoverageInvalidationOutcome::Cancelled);
        }
        let row_ids = sqlx::query_scalar::<_, i64>(
            "SELECT rowid FROM timeseries_minute_projection_v2 WHERE coverage_state <> 'warming' ORDER BY rowid LIMIT ?1",
        )
        .bind(TIMESERIES_MINUTE_PROJECTION_INVALIDATION_ROW_BATCH_LIMIT)
        .fetch_all(&state.pool)
        .await?;
        if row_ids.is_empty() {
            return Ok(TimeseriesMinuteProjectionCoverageInvalidationOutcome::Invalidated(stats));
        }

        let admission = match try_acquire_timeseries_minute_projection_write(
            coordinator,
            crate::db_pressure::global_db_pressure_gate(),
            trigger,
            "coverage_invalidation",
            pending_event_count,
        )
        .await
        {
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
            TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
                return Ok(
                    TimeseriesMinuteProjectionCoverageInvalidationOutcome::Deferred(deferred),
                );
            }
        };
        observation.get_or_insert_with(|| {
            begin_timeseries_minute_projection_observation(trigger, managed_run_id)
        });

        let started = Instant::now();
        let mut tx = state.pool.begin().await?;
        let mut query = QueryBuilder::<Sqlite>::new(
            "UPDATE timeseries_minute_projection_v2 SET coverage_state = 'warming' WHERE rowid IN (",
        );
        {
            let mut separated = query.separated(", ");
            for row_id in &row_ids {
                separated.push_bind(row_id);
            }
        }
        query.push(")");
        let updated_rows = query.build().execute(tx.as_mut()).await?.rows_affected();
        tx.commit().await?;
        stats.row_count = stats.row_count.saturating_add(updated_rows);
        stats.transaction_count = stats.transaction_count.saturating_add(1);
        debug!(
            route = "timeseries_projection",
            builder = "minute_projection_v2",
            trigger,
            transaction_phase = "coverage_invalidation",
            transaction_row_limit = TIMESERIES_MINUTE_PROJECTION_INVALIDATION_ROW_BATCH_LIMIT,
            transaction_row_count = updated_rows,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "invalidated a bounded minute projection coverage slice"
        );
        drop(admission);
        tokio::task::yield_now().await;
    }
}

pub(crate) async fn flush_timeseries_minute_projection(
    state: &AppState,
    trigger: &'static str,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("timeseries_minute_projection")
    else {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred {
                retry_after: Some(Duration::from_secs(1)),
            },
        ));
    };
    flush_timeseries_minute_projection_with_coordinator_and_cancellation(
        state,
        trigger,
        &crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
        None,
        None,
    )
    .await
}

pub(crate) async fn flush_timeseries_minute_projection_with_coordinator(
    state: &AppState,
    trigger: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    let Some(_execution_lease) =
        crate::maintenance_store::try_acquire_task_execution("timeseries_minute_projection")
    else {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred {
                retry_after: Some(Duration::from_secs(1)),
            },
        ));
    };
    flush_timeseries_minute_projection_with_coordinator_and_cancellation(
        state,
        trigger,
        coordinator,
        None,
        None,
    )
    .await
}

pub(crate) async fn flush_timeseries_minute_projection_managed(
    state: &AppState,
    trigger: &'static str,
    managed_run_id: i64,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    flush_timeseries_minute_projection_with_coordinator_and_cancellation(
        state,
        trigger,
        &crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
        None,
        Some(managed_run_id),
    )
    .await
}

pub(crate) async fn flush_timeseries_minute_projection_with_coordinator_and_cancellation(
    state: &AppState,
    trigger: &'static str,
    coordinator: &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    cancellation: Option<&tokio_util::sync::CancellationToken>,
    managed_run_id: Option<i64>,
) -> Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> {
    if timeseries_minute_projection_is_cancelled(cancellation) {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
    }
    if timeseries_minute_projection_recovery_pending(&state.pool).await? {
        let startup_cancellation = cancellation
            .cloned()
            .unwrap_or_else(tokio_util::sync::CancellationToken::new);
        let recovery = prepare_timeseries_minute_projection_after_restart_for_managed_run(
            state,
            &startup_cancellation,
            managed_run_id,
        )
        .await?;
        if recovery != TimeseriesMinuteProjectionFlushOutcome::Flushed {
            return Ok(recovery);
        }
    }
    let pending = state
        .terminal_projection_hub
        .pending_timeseries_deltas(10_000);
    let coverage_invalidation_generation = state
        .terminal_projection_hub
        .timeseries_coverage_invalidation_pending();
    if pending.is_empty() && coverage_invalidation_generation.is_none() {
        return Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed);
    }
    let memory_baseline = state.memory_diagnostics.begin_operation(state).await;
    let mut loaded_row_count = 0u64;
    let mut observation = None;
    let result: Result<TimeseriesMinuteProjectionFlushOutcome, ApiError> = async {
        let started = Instant::now();
        let mut grouped = HashMap::new();
        let mut flushed_event_ids = Vec::with_capacity(pending.len());
        for (event_id, row_id, delta) in pending {
            flushed_event_ids.push(event_id);
            add_timeseries_delta_projection_keys(&mut grouped, row_id, delta);
        }

        let coverage_invalidation = if coverage_invalidation_generation.is_some() {
            match invalidate_timeseries_minute_projection_coverage(
                state,
                coordinator,
                trigger,
                flushed_event_ids.len(),
                cancellation,
                managed_run_id,
                &mut observation,
            )
            .await?
            {
                TimeseriesMinuteProjectionCoverageInvalidationOutcome::Invalidated(stats) => stats,
                TimeseriesMinuteProjectionCoverageInvalidationOutcome::Deferred(deferred) => {
                    return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred));
                }
                TimeseriesMinuteProjectionCoverageInvalidationOutcome::Cancelled => {
                    return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
                }
            }
        } else {
            TimeseriesMinuteProjectionCoverageInvalidationStats::default()
        };

        let mut grouped = grouped.into_iter().collect::<Vec<_>>();
        grouped.sort_by_key(|(key, _)| {
            (
                key.minute_start_epoch,
                key.source_scope,
                key.upstream_account_key,
            )
        });
        let mut work_batches = Vec::<Vec<_>>::new();
        let mut work_batch = Vec::new();
        let mut work_batch_delta_count = 0usize;
        for (key, mut deltas) in grouped {
            deltas.sort_by_key(|(row_id, _)| *row_id);
            for delta_batch in deltas.chunks(TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT) {
                if !work_batch.is_empty()
                    && (work_batch.len() >= TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT
                        || work_batch_delta_count + delta_batch.len()
                            > TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT)
                {
                    work_batches.push(std::mem::take(&mut work_batch));
                    work_batch_delta_count = 0;
                }
                work_batch.push((key.clone(), delta_batch.to_vec()));
                work_batch_delta_count += delta_batch.len();
            }
        }
        if !work_batch.is_empty() {
            work_batches.push(work_batch);
        }
        let mut written_key_count = 0usize;
        let mut exact_fallback_minute_count = 0usize;
        let mut transaction_count = 0usize;
        for key_batch in work_batches {
            if timeseries_minute_projection_is_cancelled(cancellation) {
                return Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled);
            }
            let transaction_key_count = key_batch.len();
            let transaction_delta_count = key_batch
                .iter()
                .map(|(_, deltas)| deltas.len())
                .sum::<usize>();
            // Exact rebuilds can scan a hot minute with an unbounded source-row count. Keep
            // that read, aggregate construction, and JSON serialization outside P2 admission so
            // a newly-arrived P1 terminal writer only waits for the prepared SQLite upserts.
            let source_snapshot_id =
                resolve_invocation_snapshot_id(&state.pool, InvocationSourceScope::All).await?;
            let mut prepared_writes = Vec::with_capacity(transaction_key_count);
            for (key, deltas) in &key_batch {
                let (aggregate, max_row_id) = if let Some((aggregate, existing_max_row_id)) =
                    load_timeseries_minute_projection_v2_key(&state.pool, key).await?
                {
                    if timeseries_projection_requires_exact_rebuild(deltas, existing_max_row_id) {
                        // A terminal record can update an older running row. Its row ID is not a
                        // change cursor, and repeated pending events can share the same row ID.
                        // Either case needs an exact rebuild rather than another incremental add.
                        exact_fallback_minute_count += 1;
                        let (aggregate, max_row_id, source_row_count) =
                            rebuild_timeseries_minute_projection_v2_key(
                                &state.pool,
                                key,
                                Some(source_snapshot_id),
                            )
                            .await?;
                        loaded_row_count = loaded_row_count.saturating_add(source_row_count);
                        (aggregate, max_row_id)
                    } else {
                        let mut aggregate = aggregate;
                        let mut max_row_id = existing_max_row_id;
                        for (row_id, delta) in deltas {
                            add_timeseries_terminal_delta_to_aggregate(&mut aggregate, delta);
                            max_row_id = max_row_id.max(*row_id);
                        }
                        (aggregate, max_row_id)
                    }
                } else {
                    // A delta alone cannot prove that a minute is complete: a process can start
                    // mid-minute or recover after an earlier terminal write. Rebuild only this
                    // minute before publishing it as ready, rather than storing a partial total.
                    exact_fallback_minute_count += 1;
                    let (aggregate, max_row_id, source_row_count) =
                        rebuild_timeseries_minute_projection_v2_key(
                            &state.pool,
                            key,
                            Some(source_snapshot_id),
                        )
                        .await?;
                    loaded_row_count = loaded_row_count.saturating_add(source_row_count);
                    (aggregate, max_row_id)
                };
                prepared_writes.push(prepare_timeseries_minute_projection_v2_write(
                    key,
                    &aggregate,
                    max_row_id,
                )?);
            }
            let admission = match try_acquire_timeseries_minute_projection_write(
                coordinator,
                crate::db_pressure::global_db_pressure_gate(),
                trigger,
                "minute_projection_keys",
                flushed_event_ids.len(),
            )
            .await
            {
                TimeseriesMinuteProjectionWriteAdmissionOutcome::Acquired(admission) => admission,
                TimeseriesMinuteProjectionWriteAdmissionOutcome::Deferred(deferred) => {
                    return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred));
                }
            };
            observation.get_or_insert_with(|| {
                begin_timeseries_minute_projection_observation(trigger, managed_run_id)
            });

            let transaction_started = Instant::now();
            let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
            let durable_recovery_pending = sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM timeseries_minute_projection_v2_recovery WHERE consumer = ?1 AND invalidation_pending = 1)",
            )
            .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
            .fetch_one(tx.as_mut())
            .await?
                != 0;
            let new_coverage_invalidation = state
                .terminal_projection_hub
                .timeseries_coverage_invalidation_pending();
            if durable_recovery_pending
                || resolve_invocation_snapshot_id_tx(tx.as_mut(), InvocationSourceScope::All)
                    .await?
                    != source_snapshot_id
                || new_coverage_invalidation != coverage_invalidation_generation
            {
                return Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(
                    TimeseriesMinuteProjectionDeferred { retry_after: None },
                ));
            }
            for write in &prepared_writes {
                upsert_timeseries_minute_projection_v2_prepared_write_tx(tx.as_mut(), write)
                    .await?;
                written_key_count += 1;
            }
            tx.commit().await?;
            transaction_count += 1;
            debug!(
                route = "timeseries_projection",
                builder = "minute_projection_v2",
                trigger,
                transaction_phase = "minute_projection_keys",
                transaction_key_limit = TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT,
                transaction_key_count,
                transaction_delta_limit = TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT,
                transaction_delta_count,
                elapsed_ms = transaction_started.elapsed().as_millis() as u64,
                "flushed a bounded minute projection key slice"
            );
            drop(admission);
            tokio::task::yield_now().await;
        }

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
            raw_row_count = loaded_row_count,
            coverage_invalidation_row_count = coverage_invalidation.row_count,
            coverage_invalidation_transaction_count = coverage_invalidation.transaction_count,
            write_transaction_count = transaction_count,
            coverage_invalidation_pending = coverage_invalidation_generation.is_some(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "flushed terminal deltas into minute projection"
        );
        Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed)
    }
    .await;
    state
        .memory_diagnostics
        .observe_operation(
            state,
            "timeseries_minute_projection_flush",
            memory_baseline,
            loaded_row_count,
            true,
        )
        .await;
    let final_result = match result {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            let gate = crate::db_pressure::global_db_pressure_gate();
            if let Some(deferred) = timeseries_minute_projection_pressure_deferred(
                gate,
                "timeseries_minute_projection_flush",
                &error,
            ) {
                debug!(
                    route = "timeseries_projection",
                    builder = "minute_projection_v2",
                    trigger,
                    defer_reason = "sqlite_pressure",
                    retry_after_ms = deferred.retry_after.map(|value| value.as_millis() as u64),
                    "minute projection flush deferred after a database pressure error"
                );
                Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred))
            } else {
                Err(error)
            }
        }
    };
    if managed_run_id.is_none()
        && let Some(observation) = observation
    {
        let status = match &final_result {
            Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed) => "success",
            Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(_)) => "skipped",
            Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled) => "interrupted",
            Err(_) => "failed",
        };
        observation.finish_with_status(status);
    }
    final_result
}

pub(crate) fn spawn_timeseries_minute_projection_supervisor(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let pressure_gate = crate::db_pressure::global_db_pressure_gate();
        loop {
            if crate::maintenance_store::legacy_worker_should_skip("timeseries_minute_projection")
                .await
            {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => continue,
                }
            }
            let Some(_execution_lease) = crate::maintenance_store::try_acquire_task_execution(
                "timeseries_minute_projection",
            ) else {
                continue;
            };
            match prepare_timeseries_minute_projection_after_restart(state.as_ref(), &cancel).await
            {
                Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed) => break,
                Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred)) => {
                    let observed_eligibility_generation = pressure_gate.eligibility_generation();
                    debug!(
                        route = "timeseries_projection",
                        builder = "minute_projection_v2",
                        trigger = "startup_recovery",
                        observed_eligibility_generation,
                        "startup minute projection recovery yielded to higher-priority database work"
                    );
                    if let Some(retry_after) = deferred.retry_after {
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                            _ = tokio::time::sleep(retry_after) => {}
                        }
                    } else {
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        }
                    }
                }
                Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled) => return,
                Err(error) => {
                    let pressure_deferred = timeseries_minute_projection_pressure_deferred(
                        pressure_gate,
                        "timeseries_minute_projection_startup",
                        &error,
                    );
                    let observed_eligibility_generation = pressure_gate.eligibility_generation();
                    warn!(
                        route = "timeseries_projection",
                        builder = "minute_projection_v2",
                        pressure_deferred = pressure_deferred.is_some(),
                        ?error,
                        "failed to invalidate minute projection during startup recovery"
                    );
                    if let Some(deferred) = pressure_deferred {
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                            _ = tokio::time::sleep(deferred.retry_after.expect("pressure deferral has a retry deadline")) => {}
                        }
                    } else {
                        tokio::select! {
                            _ = cancel.cancelled() => return,
                            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        }
                    }
                }
            }
        }
        state
            .terminal_projection_hub
            .activate_timeseries_consumer(0);
        let mut ticker = interval(Duration::from_secs(60));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await;
        let mut deferred_eligibility_generation = None;
        let mut deferred_retry_after = None;
        loop {
            if let Some(observed_eligibility_generation) = deferred_eligibility_generation.take() {
                if let Some(retry_after) = deferred_retry_after.take() {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = ticker.tick() => {}
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                        _ = tokio::time::sleep(retry_after) => {}
                    }
                } else {
                    tokio::select! {
                        _ = cancel.cancelled() => return,
                        _ = ticker.tick() => {}
                        _ = pressure_gate.wait_for_eligibility_change(observed_eligibility_generation) => {}
                    }
                }
            } else {
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = ticker.tick() => {}
                }
            }

            let Some(_execution_lease) = crate::maintenance_store::try_acquire_task_execution(
                "timeseries_minute_projection",
            ) else {
                continue;
            };
            match flush_timeseries_minute_projection_with_coordinator_and_cancellation(
                state.as_ref(),
                "terminal_deadline",
                &crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
                Some(&cancel),
                None,
            )
            .await
            {
                Ok(TimeseriesMinuteProjectionFlushOutcome::Flushed) => {}
                Ok(TimeseriesMinuteProjectionFlushOutcome::Deferred(deferred)) => {
                    let observed_eligibility_generation = pressure_gate.eligibility_generation();
                    deferred_eligibility_generation = Some(observed_eligibility_generation);
                    deferred_retry_after = deferred.retry_after;
                    debug!(
                        route = "timeseries_projection",
                        builder = "minute_projection_v2",
                        trigger = "terminal_deadline",
                        flush_outcome = "deferred",
                        observed_eligibility_generation,
                        retry_after_ms = deferred.retry_after.map(|value| value.as_millis() as u64),
                        "minute projection flush yielded to higher-priority database work"
                    );
                }
                Ok(TimeseriesMinuteProjectionFlushOutcome::Cancelled) => return,
                Err(error) => {
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
    })
}
