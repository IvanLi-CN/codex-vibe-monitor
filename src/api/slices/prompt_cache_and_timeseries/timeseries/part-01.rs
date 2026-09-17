#[cfg(test)]
#[derive(sqlx::FromRow)]
struct TimeseriesMinuteProjectionRow {
    minute_start_epoch: i64,
    records_json: String,
    max_row_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
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

struct TimeseriesMinuteProjectionV2Load {
    aggregates: BTreeMap<i64, BucketAggregate>,
    cursor: i64,
    coverage_rows: Vec<TimeseriesMinuteProjectionV2Row>,
}

const TIMESERIES_MINUTE_PROJECTION_WRITE_KEY_BATCH_LIMIT: usize = 64;
const TIMESERIES_MINUTE_PROJECTION_WRITE_DELTA_BATCH_LIMIT: usize = 512;
const TIMESERIES_MINUTE_PROJECTION_INVALIDATION_ROW_BATCH_LIMIT: i64 = 256;
const TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER: &str = "timeseries_minute_v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimeseriesMinuteProjectionFlushOutcome {
    Flushed,
    Deferred(TimeseriesMinuteProjectionDeferred),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TimeseriesMinuteProjectionDeferred {
    retry_after: Option<Duration>,
}

enum TimeseriesMinuteProjectionWriteAdmissionOutcome {
    Acquired(TimeseriesMinuteProjectionWriteAdmission),
    Deferred(TimeseriesMinuteProjectionDeferred),
}

struct TimeseriesMinuteProjectionWriteAdmission {
    _write_permit: crate::proxy_sqlite_write_coordinator::ProxySqliteWritePermit,
    _pressure_permit: crate::db_pressure::DbBackgroundPermit,
}

#[derive(Debug, Default)]
struct TimeseriesMinuteProjectionCoverageInvalidationStats {
    row_count: u64,
    transaction_count: u64,
}

enum TimeseriesMinuteProjectionCoverageInvalidationOutcome {
    Invalidated(TimeseriesMinuteProjectionCoverageInvalidationStats),
    Deferred(TimeseriesMinuteProjectionDeferred),
    Cancelled,
}

fn timeseries_minute_projection_is_cancelled(
    cancellation: Option<&tokio_util::sync::CancellationToken>,
) -> bool {
    cancellation.is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
}

fn timeseries_minute_projection_pressure_deferred(
    pressure_gate: &crate::db_pressure::DbPressureGate,
    task: &'static str,
    error: &ApiError,
) -> Option<TimeseriesMinuteProjectionDeferred> {
    let error = match error {
        ApiError::BadRequest(error) | ApiError::Unavailable(error) | ApiError::Internal(error) => {
            error
        }
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

async fn timeseries_minute_projection_non_proxy_terminal_replacement_fence_is_installed(
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

async fn timeseries_minute_projection_recovery_generation(
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

async fn timeseries_minute_projection_recovery_pending(
    pool: &Pool<Sqlite>,
) -> Result<bool, ApiError> {
    Ok(timeseries_minute_projection_recovery_generation(pool)
        .await?
        .is_some())
}

async fn timeseries_minute_projection_v2_has_warming_coverage(
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
) -> Result<Option<TimeseriesMinuteProjectionV2Load>, ApiError> {
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
        coverage_rows: rows,
    }))
}

async fn load_ready_timeseries_minute_projection_v2_rows(
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

async fn timeseries_minute_projection_v2_snapshot_is_current(
    pool: &Pool<Sqlite>,
    terminal_projection_hub: &TerminalProjectionHub,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    coverage_rows: &[TimeseriesMinuteProjectionV2Row],
) -> Result<bool, ApiError> {
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
    .is_some_and(|current_rows| current_rows == coverage_rows);
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
async fn store_timeseries_minute_projection_v2_for_test(
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

struct TimeseriesMinuteProjectionWarmCoordinatorRequest<'a> {
    pool: &'a Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &'a TerminalProjectionHub,
    trigger: &'static str,
    coordinator: &'a Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
}

async fn store_timeseries_minute_projection_v2_warm(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &TerminalProjectionHub,
    trigger: &'static str,
) -> Result<TimeseriesMinuteProjectionWarmOutcome, ApiError> {
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let outcome = store_timeseries_minute_projection_v2_warm_with_coordinator_request(
        TimeseriesMinuteProjectionWarmCoordinatorRequest {
            pool,
            start,
            end,
            source_scope,
            upstream_account_id,
            terminal_projection_hub,
            trigger,
            coordinator: &coordinator,
        },
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

async fn store_timeseries_minute_projection_v2_warm_with_eligibility_retry(
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

#[cfg(not(test))]
pub(crate) async fn store_timeseries_minute_projection_v2_warm_with_coordinator(
    pool: &Pool<Sqlite>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &TerminalProjectionHub,
    trigger_and_coordinator: (
        &'static str,
        &Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
    ),
) -> Result<TimeseriesMinuteProjectionWarmOutcome, ApiError> {
    let (trigger, coordinator) = trigger_and_coordinator;
    store_timeseries_minute_projection_v2_warm_with_coordinator_request(
        TimeseriesMinuteProjectionWarmCoordinatorRequest {
            pool,
            start,
            end,
            source_scope,
            upstream_account_id,
            terminal_projection_hub,
            trigger,
            coordinator,
        },
    )
    .await
}

#[cfg(test)]
macro_rules! define_test_timeseries_minute_projection_warm_with_coordinator {
    ($lint:path) => {
        #[expect(
            $lint,
            reason = "Test compatibility adapter preserves established stateful test call sites."
        )]
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
            store_timeseries_minute_projection_v2_warm_with_coordinator_request(
                TimeseriesMinuteProjectionWarmCoordinatorRequest {
                    pool,
                    start,
                    end,
                    source_scope,
                    upstream_account_id,
                    terminal_projection_hub,
                    trigger,
                    coordinator,
                },
            )
            .await
        }
    };
}

#[cfg(test)]
define_test_timeseries_minute_projection_warm_with_coordinator!(clippy::too_many_arguments);

async fn store_timeseries_minute_projection_v2_warm_with_coordinator_request(
    request: TimeseriesMinuteProjectionWarmCoordinatorRequest<'_>,
) -> Result<TimeseriesMinuteProjectionWarmOutcome, ApiError> {
    let TimeseriesMinuteProjectionWarmCoordinatorRequest {
        pool,
        start,
        end,
        source_scope,
        upstream_account_id,
        terminal_projection_hub,
        trigger,
        coordinator,
    } = request;
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
        let outcome =
            warm_timeseries_minute_projection_batch(TimeseriesMinuteProjectionWarmBatchRequest {
                pool,
                key_batch,
                source_scope,
                upstream_account_id,
                terminal_projection_hub,
                projection_selection,
                trigger,
                coordinator,
            })
            .await?;
        if let Some(outcome) = outcome {
            return Ok(outcome);
        }
        tokio::task::yield_now().await;
    }
    Ok(TimeseriesMinuteProjectionWarmOutcome::Stored)
}

struct TimeseriesMinuteProjectionWarmBatchRequest<'a> {
    pool: &'a Pool<Sqlite>,
    key_batch: &'a [TimeseriesMinuteProjectionKey],
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &'a TerminalProjectionHub,
    projection_selection: TimeseriesProjectionSelection,
    trigger: &'static str,
    coordinator: &'a Arc<crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator>,
}

struct PreparedTimeseriesWarmBatch {
    source_snapshot_id: i64,
    writes: Vec<TimeseriesMinuteProjectionPreparedWrite>,
    snapshot_records: Vec<TimeseriesProjectionSnapshotRecord>,
}

async fn warm_timeseries_minute_projection_batch(
    request: TimeseriesMinuteProjectionWarmBatchRequest<'_>,
) -> Result<Option<TimeseriesMinuteProjectionWarmOutcome>, ApiError> {
    let TimeseriesMinuteProjectionWarmBatchRequest {
        pool,
        key_batch,
        source_scope,
        upstream_account_id,
        terminal_projection_hub,
        projection_selection,
        trigger,
        coordinator,
    } = request;
    if timeseries_minute_projection_recovery_pending(pool).await?
        || terminal_projection_hub
            .timeseries_coverage_invalidation_pending()
            .is_some()
    {
        return Ok(Some(TimeseriesMinuteProjectionWarmOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred { retry_after: None },
        )));
    }
    let prepared = prepare_timeseries_warm_batch(
        pool,
        key_batch,
        source_scope,
        upstream_account_id,
        terminal_projection_hub,
        projection_selection,
    )
    .await?;
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
            return Ok(Some(TimeseriesMinuteProjectionWarmOutcome::Deferred(
                deferred,
            )));
        }
    };
    if terminal_projection_hub
        .timeseries_coverage_invalidation_pending()
        .is_some()
    {
        drop(admission);
        return Ok(Some(TimeseriesMinuteProjectionWarmOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred { retry_after: None },
        )));
    }
    write_timeseries_warm_batch(WriteTimeseriesWarmBatchRequest {
        pool,
        source_scope,
        key_count: key_batch.len(),
        terminal_projection_hub,
        projection_selection,
        trigger,
        prepared,
        admission,
    })
    .await
}

async fn prepare_timeseries_warm_batch(
    pool: &Pool<Sqlite>,
    key_batch: &[TimeseriesMinuteProjectionKey],
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    terminal_projection_hub: &TerminalProjectionHub,
    projection_selection: TimeseriesProjectionSelection,
) -> Result<PreparedTimeseriesWarmBatch, ApiError> {
    let batch_start = Utc
        .timestamp_opt(key_batch[0].minute_start_epoch, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid minute projection batch start")))?;
    let batch_end = Utc
        .timestamp_opt(key_batch[key_batch.len() - 1].minute_start_epoch + 60, 0)
        .single()
        .ok_or_else(|| ApiError::from(anyhow!("invalid minute projection batch end")))?;
    let source_snapshot_id = resolve_invocation_snapshot_id(pool, source_scope).await?;
    let source_records = match upstream_account_id {
        Some(account_id) => {
            query_invocation_aggregate_records_from_live_range_for_account(
                pool,
                ExactUtcRange {
                    start: batch_start,
                    end: batch_end,
                },
                source_scope,
                None,
                Some(source_snapshot_id),
                account_id,
            )
            .await?
        }
        None => {
            query_invocation_aggregate_records_from_live_range(
                pool,
                ExactUtcRange {
                    start: batch_start,
                    end: batch_end,
                },
                source_scope,
                None,
                Some(source_snapshot_id),
            )
            .await?
        }
    };
    let pending = terminal_projection_hub
        .pending_timeseries_deltas_for_selection(projection_selection, 10_000);
    let snapshot_records = timeseries_projection_snapshot_records(&source_records, &pending);
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
        add_exact_record_to_timeseries_aggregate(aggregates.entry(minute).or_default(), &record);
        max_row_ids
            .entry(minute)
            .and_modify(|max_row_id| *max_row_id = (*max_row_id).max(record.id))
            .or_insert(record.id);
    }
    let writes = key_batch
        .iter()
        .map(|key| {
            let aggregate = aggregates
                .remove(&key.minute_start_epoch)
                .unwrap_or_default();
            let max_row_id = max_row_ids.remove(&key.minute_start_epoch).unwrap_or(0);
            prepare_timeseries_minute_projection_v2_write(key, &aggregate, max_row_id)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PreparedTimeseriesWarmBatch {
        source_snapshot_id,
        writes,
        snapshot_records,
    })
}

struct WriteTimeseriesWarmBatchRequest<'a> {
    pool: &'a Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    key_count: usize,
    terminal_projection_hub: &'a TerminalProjectionHub,
    projection_selection: TimeseriesProjectionSelection,
    trigger: &'static str,
    prepared: PreparedTimeseriesWarmBatch,
    admission: TimeseriesMinuteProjectionWriteAdmission,
}

async fn write_timeseries_warm_batch(
    request: WriteTimeseriesWarmBatchRequest<'_>,
) -> Result<Option<TimeseriesMinuteProjectionWarmOutcome>, ApiError> {
    let WriteTimeseriesWarmBatchRequest {
        pool,
        source_scope,
        key_count,
        terminal_projection_hub,
        projection_selection,
        trigger,
        prepared,
        admission,
    } = request;
    let transaction_started = Instant::now();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let recovery_pending = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM timeseries_minute_projection_v2_recovery WHERE consumer = ?1 AND invalidation_pending = 1)",
    )
    .bind(TIMESERIES_MINUTE_PROJECTION_RECOVERY_CONSUMER)
    .fetch_one(tx.as_mut())
    .await?
        != 0;
    if recovery_pending
        || resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?
            != prepared.source_snapshot_id
    {
        return Ok(Some(TimeseriesMinuteProjectionWarmOutcome::Deferred(
            TimeseriesMinuteProjectionDeferred { retry_after: None },
        )));
    }
    for write in &prepared.writes {
        upsert_timeseries_minute_projection_v2_prepared_write_tx(tx.as_mut(), write).await?;
    }
    tx.commit().await?;
    let coverage_ack_count = terminal_projection_hub
        .mark_timeseries_warm_coverage(projection_selection, &prepared.snapshot_records);
    debug!(
        route = "timeseries_projection",
        builder = "minute_projection_v2",
        trigger,
        transaction_phase = "exact_warm",
        transaction_key_count = key_count,
        coverage_ack_count,
        elapsed_ms = transaction_started.elapsed().as_millis() as u64,
        "materialized and acknowledged a bounded exact-fallback minute projection slice"
    );
    drop(admission);
    Ok(None)
}
