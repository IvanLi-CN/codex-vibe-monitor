use super::*;
use crate::observability::contention::{
    PerformanceGauge, StatsProjection, StatsQuery, current_performance, observe_query,
};

pub(super) fn projection_interval_key(
    bucket_kind: &'static str,
    bucket_key: String,
    dimension: String,
    series_key: String,
) -> LongTermProjectionIntervalKey {
    LongTermProjectionIntervalKey {
        bucket_kind,
        bucket_key,
        dimension,
        series_key,
    }
}

pub(super) fn collect_long_term_projection_interval_segments(
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    invocation_row_id: i64,
) -> Vec<LongTermProjectionIntervalSegment> {
    let mut interval_start_ms = None;
    let mut interval_end_ms = None;
    for bucket in hourly
        .values()
        .filter(|bucket| bucket.dimension == "overall")
    {
        for &(start, end) in &bucket.accumulator.intervals {
            interval_start_ms =
                Some(interval_start_ms.map_or(start, |current: i64| current.min(start)));
            interval_end_ms = Some(interval_end_ms.map_or(end, |current: i64| current.max(end)));
        }
    }
    let (Some(interval_start_ms), Some(interval_end_ms)) = (interval_start_ms, interval_end_ms)
    else {
        return Vec::new();
    };
    let model_series_key = daily
        .values()
        .find(|bucket| bucket.dimension == "model")
        .map(|bucket| bucket.series_key.clone())
        .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string());
    let upstream_series_key = daily
        .values()
        .find(|bucket| bucket.dimension == "upstream")
        .map(|bucket| bucket.series_key.clone())
        .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string());
    vec![LongTermProjectionIntervalSegment {
        invocation_row_id,
        model_series_key,
        upstream_series_key,
        interval_start_ms,
        interval_end_ms,
    }]
}

pub(super) fn merge_long_term_projection_bucket(
    target: &mut LongTermBucket,
    source: &LongTermBucket,
) {
    target.accumulator.merge(&source.accumulator);
}

pub(super) fn merge_long_term_projection_buckets<K>(
    target: &mut HashMap<K, LongTermBucket>,
    source: HashMap<K, LongTermBucket>,
) where
    K: Eq + Hash,
{
    for (key, bucket) in source {
        match target.entry(key) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_long_term_projection_bucket(entry.get_mut(), &bucket);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(bucket);
            }
        }
    }
}

pub(super) async fn insert_long_term_hourly(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    bucket: &LongTermBucket,
) -> Result<()> {
    insert_long_term_rollup(
        tx,
        "long_term_usage_hourly",
        "bucket_start_epoch",
        bucket.bucket_start_epoch.to_string(),
        bucket,
    )
    .await
}

pub(super) async fn insert_long_term_daily(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    bucket: &LongTermBucket,
) -> Result<()> {
    let Some(date) = bucket.stats_date.as_deref() else {
        return Ok(());
    };
    insert_long_term_rollup(
        tx,
        "long_term_usage_daily",
        "stats_date",
        date.to_string(),
        bucket,
    )
    .await
}

pub(super) async fn insert_long_term_rollup(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    bucket_column: &str,
    bucket_value: String,
    bucket: &LongTermBucket,
) -> Result<()> {
    let conflict_target = if table == "long_term_usage_daily" {
        "stats_date, dimension, series_key"
    } else {
        "bucket_start_epoch, dimension, series_key"
    };
    let sql = format!(
        "INSERT INTO {table} ({bucket_column}, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21) ON CONFLICT ({conflict_target}) DO UPDATE SET display_name = excluded.display_name, reasoning_effort = excluded.reasoning_effort, calls = excluded.calls, token_total = excluded.token_total, token_samples = excluded.token_samples, cost_total = excluded.cost_total, cost_samples = excluded.cost_samples, usage_time_ms = excluded.usage_time_ms, usage_time_samples = excluded.usage_time_samples, wall_time_ms = excluded.wall_time_ms, wall_time_samples = excluded.wall_time_samples, output_tokens_total = excluded.output_tokens_total, stream_duration_ms = excluded.stream_duration_ms, output_speed_samples = excluded.output_speed_samples, first_byte_sum_ms = excluded.first_byte_sum_ms, first_byte_samples = excluded.first_byte_samples, response_sum_ms = excluded.response_sum_ms, response_samples = excluded.response_samples, updated_at = datetime('now')"
    );
    let acc = &bucket.accumulator;
    sqlx::query(&sql)
        .bind(bucket_value)
        .bind(&bucket.dimension)
        .bind(&bucket.series_key)
        .bind(&bucket.display_name)
        .bind(&bucket.reasoning_effort)
        .bind(acc.calls)
        .bind(acc.token_total)
        .bind(acc.token_samples)
        .bind(acc.cost_total)
        .bind(acc.cost_samples)
        .bind(acc.usage_time_ms)
        .bind(acc.usage_time_samples)
        .bind(acc.wall_time_ms())
        .bind(acc.wall_sample_count())
        .bind(acc.output_tokens_total)
        .bind(acc.stream_duration_ms)
        .bind(acc.output_speed_samples)
        .bind(acc.first_byte_sum_ms)
        .bind(acc.first_byte_samples)
        .bind(acc.response_sum_ms)
        .bind(acc.response_samples)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn load_long_term_projection_interval_index(
    pool: &Pool<Sqlite>,
    dates: &HashSet<String>,
) -> Result<HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>> {
    if dates.is_empty() {
        return Ok(HashMap::new());
    }
    let requested_dates = dates
        .iter()
        .filter_map(|date| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .ok()
                .map(|value| (date, value))
        })
        .collect::<Vec<_>>();
    if requested_dates.is_empty() {
        return Ok(HashMap::new());
    }
    let mut suppressed_builder = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_row_id, bucket_date FROM long_term_projection_interval_suppressions WHERE bucket_date IN (",
    );
    let mut suppressed_dates = suppressed_builder.separated(", ");
    for (date, _) in &requested_dates {
        suppressed_dates.push_bind(*date);
    }
    suppressed_dates.push_unseparated(")");
    let suppressed = suppressed_builder
        .build_query_as::<(i64, String)>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect::<HashSet<_>>();
    let first_start_ms = requested_dates
        .iter()
        .filter_map(|(_, date)| long_term_day_epoch_bounds(*date).map(|(start, _)| start * 1_000))
        .min()
        .context("invalid long-term projection interval date range")?;
    let last_end_ms = requested_dates
        .iter()
        .filter_map(|(_, date)| long_term_day_epoch_bounds(*date).map(|(_, end)| end * 1_000))
        .max()
        .context("invalid long-term projection interval date range")?;
    let canonical_rows = observe_query(
        StatsQuery::LongTermOverlap,
        sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE interval_start_ms < ?1 AND interval_end_ms > ?2",
        )
        .bind(last_end_ms)
        .bind(first_start_ms)
        .fetch_all(pool),
    )
    .await?;
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_row_id, bucket_kind, bucket_date, bucket_key, dimension, series_key, interval_start_ms, interval_end_ms FROM long_term_projection_intervals legacy WHERE bucket_date IN (",
    );
    let mut separated = builder.separated(", ");
    for date in dates {
        separated.push_bind(date);
    }
    separated.push_unseparated(") AND NOT EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = legacy.invocation_row_id)");
    let rows = builder
        .build_query_as::<LongTermProjectionLegacyIntervalRow>()
        .fetch_all(pool)
        .await?;
    let mut index = HashMap::new();
    for state in canonical_rows {
        for (date_text, date) in &requested_dates {
            if !suppressed.contains(&(state.invocation_row_id, (*date_text).to_string())) {
                add_long_term_projection_interval_state_for_date(&mut index, *date, &state);
            }
        }
    }
    for row in rows {
        if suppressed.contains(&(row.invocation_row_id, row.bucket_date.clone())) {
            continue;
        }
        let bucket_kind = match row.bucket_kind.as_str() {
            "hourly" => "hourly",
            "daily" => "daily",
            _ => continue,
        };
        index
            .entry(projection_interval_key(
                bucket_kind,
                row.bucket_key,
                row.dimension,
                row.series_key,
            ))
            .or_insert_with(LongTermProjectionIntervalUnion::default)
            .add(row.interval_start_ms, row.interval_end_ms);
    }
    Ok(index)
}

pub(super) fn add_long_term_projection_interval_state_for_date(
    index: &mut HashMap<LongTermProjectionIntervalKey, LongTermProjectionIntervalUnion>,
    date: NaiveDate,
    state: &LongTermProjectionIntervalStateRow,
) {
    let Some((day_start_epoch, day_end_epoch)) = long_term_day_epoch_bounds(date) else {
        return;
    };
    let day_start_ms = day_start_epoch * 1_000;
    let day_end_ms = day_end_epoch * 1_000;
    let clipped_start_ms = state.interval_start_ms.max(day_start_ms);
    let clipped_end_ms = state.interval_end_ms.min(day_end_ms);
    if clipped_end_ms <= clipped_start_ms {
        return;
    }
    for (dimension, series_key) in [
        ("overall", "overall"),
        ("model", state.model_series_key.as_str()),
        ("upstream", state.upstream_series_key.as_str()),
    ] {
        index
            .entry(projection_interval_key(
                "daily",
                date.to_string(),
                dimension.to_string(),
                series_key.to_string(),
            ))
            .or_default()
            .add(clipped_start_ms, clipped_end_ms);
    }
    let mut hour_start_ms = clipped_start_ms.div_euclid(LONG_TERM_HOUR_MS) * LONG_TERM_HOUR_MS;
    while hour_start_ms < clipped_end_ms {
        let hour_end_ms = hour_start_ms.saturating_add(LONG_TERM_HOUR_MS);
        let hour_clipped_start_ms = clipped_start_ms.max(hour_start_ms);
        let hour_clipped_end_ms = clipped_end_ms.min(hour_end_ms);
        for (dimension, series_key) in [
            ("overall", "overall"),
            ("model", state.model_series_key.as_str()),
            ("upstream", state.upstream_series_key.as_str()),
        ] {
            index
                .entry(projection_interval_key(
                    "hourly",
                    (hour_start_ms / 1_000).to_string(),
                    dimension.to_string(),
                    series_key.to_string(),
                ))
                .or_default()
                .add(hour_clipped_start_ms, hour_clipped_end_ms);
        }
        hour_start_ms = hour_end_ms;
    }
}

pub(super) fn long_term_projection_interval_dates(
    segment: &LongTermProjectionIntervalSegment,
) -> HashSet<String> {
    let mut dates = HashSet::new();
    let Some(mut date) = Shanghai
        .timestamp_millis_opt(segment.interval_start_ms)
        .single()
        .map(|timestamp| timestamp.date_naive())
    else {
        return dates;
    };
    while let Some((_, day_end_epoch)) = long_term_day_epoch_bounds(date) {
        dates.insert(date.to_string());
        if day_end_epoch.saturating_mul(1_000) >= segment.interval_end_ms {
            break;
        }
        let Some(next) = date.succ_opt() else {
            break;
        };
        date = next;
    }
    dates
}

pub(super) async fn upsert_long_term_projection_interval_segments(
    pool: &Pool<Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<()> {
    for batch in segments.chunks(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS) {
        let (mut transaction, permit) = control.begin(pool).await?;
        upsert_long_term_projection_interval_segments_in_transaction(&mut transaction, batch)
            .await?;
        control.commit(transaction, permit).await?;
    }
    Ok(())
}

pub(super) async fn upsert_long_term_projection_interval_segments_in_transaction(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
) -> Result<()> {
    for segment in segments {
        sqlx::query(
            "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(invocation_row_id) DO UPDATE SET model_series_key = excluded.model_series_key, upstream_series_key = excluded.upstream_series_key, interval_start_ms = excluded.interval_start_ms, interval_end_ms = excluded.interval_end_ms",
        )
        .bind(segment.invocation_row_id)
        .bind(&segment.model_series_key)
        .bind(&segment.upstream_series_key)
        .bind(segment.interval_start_ms)
        .bind(segment.interval_end_ms)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

pub(super) async fn load_long_term_projection_interval_state_ids(
    pool: &Pool<Sqlite>,
    segments: &[LongTermProjectionIntervalSegment],
) -> Result<HashSet<i64>> {
    if segments.is_empty() {
        return Ok(HashSet::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT invocation_row_id FROM long_term_projection_interval_state WHERE invocation_row_id IN (",
    );
    let mut ids = builder.separated(", ");
    for segment in segments {
        ids.push_bind(segment.invocation_row_id);
    }
    ids.push_unseparated(")");
    Ok(builder
        .build_query_scalar::<i64>()
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect())
}

pub(super) async fn migrate_long_term_projection_legacy_interval_state(
    pool: &Pool<Sqlite>,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<bool> {
    let legacy_rows = sqlx::query_as::<_, LongTermProjectionLegacyCompactRow>(
        r#"
        SELECT legacy.invocation_row_id,
               MAX(CASE WHEN legacy.dimension = 'model' THEN legacy.series_key END) AS model_series_key,
               MAX(CASE WHEN legacy.dimension = 'upstream' THEN legacy.series_key END) AS upstream_series_key,
               MIN(legacy.interval_start_ms) AS interval_start_ms,
               MAX(legacy.interval_end_ms) AS interval_end_ms
        FROM long_term_projection_intervals legacy
        WHERE NOT EXISTS (
            SELECT 1
            FROM long_term_projection_interval_state state
            WHERE state.invocation_row_id = legacy.invocation_row_id
        )
        GROUP BY legacy.invocation_row_id
        ORDER BY legacy.invocation_row_id ASC
        LIMIT ?1
        "#,
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .fetch_all(pool)
    .await?;
    if !legacy_rows.is_empty() {
        let segments = legacy_rows
            .into_iter()
            .filter(|row| row.interval_end_ms > row.interval_start_ms)
            .map(|row| LongTermProjectionIntervalSegment {
                invocation_row_id: row.invocation_row_id,
                model_series_key: row
                    .model_series_key
                    .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string()),
                upstream_series_key: row
                    .upstream_series_key
                    .unwrap_or_else(|| LONG_TERM_OTHER_KEY.to_string()),
                interval_start_ms: row.interval_start_ms,
                interval_end_ms: row.interval_end_ms,
            })
            .collect::<Vec<_>>();
        if segments.is_empty() {
            return Ok(false);
        }
        upsert_long_term_projection_interval_segments(pool, &segments, control).await?;
        return Ok(true);
    }

    let cleanup_pending = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_intervals legacy WHERE EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = legacy.invocation_row_id) LIMIT 1)",
    )
    .fetch_one(pool)
    .await?
        != 0;
    if !cleanup_pending {
        return Ok(false);
    }
    let (mut transaction, permit) = control.begin(pool).await?;
    let deleted = sqlx::query(
        "DELETE FROM long_term_projection_intervals WHERE rowid IN (SELECT legacy.rowid FROM long_term_projection_intervals legacy WHERE EXISTS (SELECT 1 FROM long_term_projection_interval_state state WHERE state.invocation_row_id = legacy.invocation_row_id) LIMIT ?1)",
    )
    .bind(LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    control.commit(transaction, permit).await?;
    Ok(deleted != 0)
}

pub(super) async fn merge_long_term_projection_rollup(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    table: &str,
    bucket_column: &str,
    bucket_value: String,
    bucket: &LongTermBucket,
    interval_union: Option<&LongTermProjectionIntervalUnion>,
) -> Result<()> {
    let conflict_target = if table == "long_term_usage_daily" {
        "stats_date, dimension, series_key"
    } else {
        "bucket_start_epoch, dimension, series_key"
    };
    let sql = format!(
        "INSERT INTO {table} ({bucket_column}, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21) ON CONFLICT ({conflict_target}) DO UPDATE SET display_name = excluded.display_name, reasoning_effort = excluded.reasoning_effort, calls = calls + excluded.calls, token_total = token_total + excluded.token_total, token_samples = token_samples + excluded.token_samples, cost_total = cost_total + excluded.cost_total, cost_samples = cost_samples + excluded.cost_samples, usage_time_ms = usage_time_ms + excluded.usage_time_ms, usage_time_samples = usage_time_samples + excluded.usage_time_samples, wall_time_ms = excluded.wall_time_ms, wall_time_samples = excluded.wall_time_samples, output_tokens_total = output_tokens_total + excluded.output_tokens_total, stream_duration_ms = stream_duration_ms + excluded.stream_duration_ms, output_speed_samples = output_speed_samples + excluded.output_speed_samples, first_byte_sum_ms = first_byte_sum_ms + excluded.first_byte_sum_ms, first_byte_samples = first_byte_samples + excluded.first_byte_samples, response_sum_ms = response_sum_ms + excluded.response_sum_ms, response_samples = response_samples + excluded.response_samples, updated_at = datetime('now')"
    );
    let acc = &bucket.accumulator;
    sqlx::query(&sql)
        .bind(bucket_value)
        .bind(&bucket.dimension)
        .bind(&bucket.series_key)
        .bind(&bucket.display_name)
        .bind(&bucket.reasoning_effort)
        .bind(acc.calls)
        .bind(acc.token_total)
        .bind(acc.token_samples)
        .bind(acc.cost_total)
        .bind(acc.cost_samples)
        .bind(acc.usage_time_ms)
        .bind(acc.usage_time_samples)
        .bind(interval_union.map_or(0, |value| value.duration_ms) as f64)
        .bind(interval_union.map_or(0, |value| value.sample_count))
        .bind(acc.output_tokens_total)
        .bind(acc.stream_duration_ms)
        .bind(acc.output_speed_samples)
        .bind(acc.first_byte_sum_ms)
        .bind(acc.first_byte_samples)
        .bind(acc.response_sum_ms)
        .bind(acc.response_samples)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn apply_long_term_projection_incremental(
    state: &AppState,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    segments: &[LongTermProjectionIntervalSegment],
    next_cursor: i64,
    event_count: usize,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::background(
        &state.shutdown,
        crate::db_pressure::global_db_pressure_gate(),
    );
    let _ = apply_long_term_projection_incremental_with_runtime_and_control(
        &state.pool,
        &state.long_term_projection_runtime,
        LongTermProjectionIncrementalBatch {
            hourly,
            daily,
            segments,
        },
        next_cursor,
        event_count,
        &control,
    )
    .await?;
    Ok(())
}

pub(super) async fn apply_long_term_projection_incremental_with_runtime(
    pool: &Pool<Sqlite>,
    runtime: &Arc<Mutex<LongTermProjectionRuntime>>,
    hourly: &HashMap<(i64, String, String), LongTermBucket>,
    daily: &HashMap<(String, String, String), LongTermBucket>,
    segments: &[LongTermProjectionIntervalSegment],
    next_cursor: i64,
    event_count: usize,
) -> Result<()> {
    let control = LongTermProjectionWriteControl::unrestricted();
    let _ = apply_long_term_projection_incremental_with_runtime_and_control(
        pool,
        runtime,
        LongTermProjectionIncrementalBatch {
            hourly,
            daily,
            segments,
        },
        next_cursor,
        event_count,
        &control,
    )
    .await?;
    Ok(())
}

pub(super) struct LongTermProjectionIncrementalBatch<'a> {
    pub(super) hourly: &'a HashMap<(i64, String, String), LongTermBucket>,
    pub(super) daily: &'a HashMap<(String, String, String), LongTermBucket>,
    pub(super) segments: &'a [LongTermProjectionIntervalSegment],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LongTermProjectionIncrementalOutcome {
    Published,
    RebuildRequired,
}

pub(super) async fn long_term_projection_incremental_dates_are_dirty(
    transaction: &mut sqlx::Transaction<'_, Sqlite>,
    dates: &HashSet<String>,
) -> Result<bool> {
    if dates.is_empty() {
        return Ok(false);
    }
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT EXISTS(SELECT 1 FROM long_term_projection_dirty_buckets WHERE bucket_date IN (",
    );
    {
        let mut separated = query.separated(", ");
        for date in dates {
            separated.push_bind(date);
        }
    }
    query.push("))");
    Ok(query
        .build_query_scalar::<i64>()
        .fetch_one(&mut **transaction)
        .await?
        != 0)
}

pub(super) async fn apply_long_term_projection_incremental_with_runtime_and_control(
    pool: &Pool<Sqlite>,
    runtime: &Arc<Mutex<LongTermProjectionRuntime>>,
    batch: LongTermProjectionIncrementalBatch<'_>,
    next_cursor: i64,
    event_count: usize,
    control: &LongTermProjectionWriteControl<'_>,
) -> Result<LongTermProjectionIncrementalOutcome> {
    let rollup_rows = batch.hourly.len() + batch.daily.len();
    let mutation_rows = rollup_rows + batch.segments.len();
    if mutation_rows > LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS {
        bail!(
            "long-term projection incremental batch has {mutation_rows} writes; maximum is {LONG_TERM_PROJECTION_INCREMENTAL_MUTATION_ROWS}"
        );
    }
    let mut dates = HashSet::new();
    for segment in batch.segments {
        dates.extend(long_term_projection_interval_dates(segment));
    }
    for bucket in batch.daily.values() {
        if let Some(date) = bucket.stats_date.as_ref() {
            dates.insert(date.clone());
        }
    }
    let (mut interval_index, mut loaded_dates) = {
        let runtime = runtime.lock().await;
        (
            runtime.interval_index.clone(),
            runtime.loaded_interval_dates.clone(),
        )
    };
    let missing_dates = dates
        .difference(&loaded_dates)
        .cloned()
        .collect::<HashSet<_>>();
    if !missing_dates.is_empty() {
        interval_index
            .extend(load_long_term_projection_interval_index(pool, &missing_dates).await?);
        loaded_dates.extend(missing_dates);
    }
    let persisted_segment_ids =
        load_long_term_projection_interval_state_ids(pool, batch.segments).await?;
    for segment in batch
        .segments
        .iter()
        .filter(|segment| !persisted_segment_ids.contains(&segment.invocation_row_id))
    {
        let state = LongTermProjectionIntervalStateRow {
            invocation_row_id: segment.invocation_row_id,
            model_series_key: segment.model_series_key.clone(),
            upstream_series_key: segment.upstream_series_key.clone(),
            interval_start_ms: segment.interval_start_ms,
            interval_end_ms: segment.interval_end_ms,
        };
        for date in &dates {
            if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                add_long_term_projection_interval_state_for_date(&mut interval_index, date, &state);
            }
        }
    }
    let (mut tx, permit) = control.begin(pool).await?;
    if long_term_projection_incremental_dates_are_dirty(&mut tx, &dates).await? {
        drop(tx);
        drop(permit);
        return Ok(LongTermProjectionIncrementalOutcome::RebuildRequired);
    }
    upsert_long_term_projection_interval_segments_in_transaction(&mut tx, batch.segments).await?;
    for bucket in batch.hourly.values() {
        let key = projection_interval_key(
            "hourly",
            bucket.bucket_start_epoch.to_string(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        merge_long_term_projection_rollup(
            &mut tx,
            "long_term_usage_hourly",
            "bucket_start_epoch",
            bucket.bucket_start_epoch.to_string(),
            bucket,
            interval_index.get(&key),
        )
        .await?;
    }
    for bucket in batch.daily.values() {
        let Some(date) = bucket.stats_date.clone() else {
            continue;
        };
        let key = projection_interval_key(
            "daily",
            date.clone(),
            bucket.dimension.clone(),
            bucket.series_key.clone(),
        );
        merge_long_term_projection_rollup(
            &mut tx,
            "long_term_usage_daily",
            "stats_date",
            date,
            bucket,
            interval_index.get(&key),
        )
        .await?;
    }
    sqlx::query(
        "INSERT INTO long_term_projection_state (consumer, cursor_row_id, last_flush_at, last_error) VALUES (?1, ?2, datetime('now'), NULL) ON CONFLICT(consumer) DO UPDATE SET cursor_row_id = excluded.cursor_row_id, last_flush_at = excluded.last_flush_at, last_error = NULL, updated_at = datetime('now')",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(next_cursor)
    .execute(&mut *tx)
    .await?;
    let statistics_start_date = batch
        .daily
        .values()
        .filter_map(|bucket| bucket.stats_date.as_deref())
        .min()
        .map(str::to_string);
    sqlx::query(
        "UPDATE long_term_stats_state SET status = CASE WHEN ?1 > 0 AND status <> ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets) THEN ?3 ELSE status END, statistics_start_date = CASE WHEN ?4 IS NULL THEN statistics_start_date WHEN statistics_start_date IS NULL OR ?4 < statistics_start_date THEN ?4 ELSE statistics_start_date END, processed_rows = processed_rows + ?1, total_rows = total_rows + ?1, last_error = CASE WHEN ?1 > 0 AND status <> ?2 AND NOT EXISTS (SELECT 1 FROM long_term_projection_dirty_buckets) THEN NULL ELSE last_error END, updated_at = datetime('now') WHERE id = ?5",
    )
    .bind(event_count as i64)
    .bind(LONG_TERM_STATUS_ERROR)
    .bind(LONG_TERM_STATUS_READY)
    .bind(statistics_start_date)
    .bind(LONG_TERM_STATE_ID)
    .execute(&mut *tx)
    .await?;
    control.commit(tx, permit).await?;
    if let Some(metrics) = current_performance().filter(|metrics| metrics.enabled) {
        metrics.set_performance(
            PerformanceGauge::ProjectionLastGood(StatsProjection::LongTerm),
            Utc::now().timestamp() as f64,
        );
    }
    let mut runtime = runtime.lock().await;
    runtime.interval_index = interval_index;
    runtime.loaded_interval_dates = loaded_dates;
    Ok(LongTermProjectionIncrementalOutcome::Published)
}

pub(super) async fn load_long_term_rollups(
    pool: &Pool<Sqlite>,
    range: LongTermRange,
    dimension: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let table = "active_daily";
    let sql = format!(
        "{} SELECT MAX(d.stats_date) AS bucket_or_date, d.dimension, d.series_key, COALESCE((SELECT account.display_name FROM pool_upstream_accounts account WHERE d.dimension = 'upstream' AND d.series_key LIKE 'account:%' AND account.id = CAST(substr(d.series_key, 9) AS INTEGER) LIMIT 1), (SELECT latest.display_name FROM {table} latest WHERE latest.dimension = ?1 AND latest.series_key = d.series_key AND latest.reasoning_effort = d.reasoning_effort AND latest.stats_date BETWEEN ?2 AND ?3 ORDER BY latest.stats_date DESC LIMIT 1)) AS display_name, d.reasoning_effort, SUM(d.calls) AS calls, SUM(d.token_total) AS token_total, SUM(d.token_samples) AS token_samples, SUM(d.cost_total) AS cost_total, SUM(d.cost_samples) AS cost_samples, SUM(d.usage_time_ms) AS usage_time_ms, SUM(d.usage_time_samples) AS usage_time_samples, SUM(d.wall_time_ms) AS wall_time_ms, SUM(d.wall_time_samples) AS wall_time_samples, SUM(d.output_tokens_total) AS output_tokens_total, SUM(d.stream_duration_ms) AS stream_duration_ms, SUM(d.output_speed_samples) AS output_speed_samples, SUM(d.first_byte_sum_ms) AS first_byte_sum_ms, SUM(d.first_byte_samples) AS first_byte_samples, SUM(d.response_sum_ms) AS response_sum_ms, SUM(d.response_samples) AS response_samples FROM {table} d WHERE d.dimension = ?1 AND d.stats_date BETWEEN ?2 AND ?3 GROUP BY d.series_key, d.reasoning_effort",
        long_term_projection_active_daily_cte(),
    );
    let _ = range;
    Ok(sqlx::query_as::<_, LongTermRollupRow>(&sql)
        .bind(dimension)
        .bind(start_date)
        .bind(end_date)
        .fetch_all(pool)
        .await?)
}

pub(super) async fn load_long_term_daily_rows(
    pool: &Pool<Sqlite>,
    dimension: &str,
    series_key: Option<&str>,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<LongTermRollupRow>> {
    let mut sql = format!(
        "{} SELECT stats_date AS bucket_or_date, dimension, series_key, display_name, reasoning_effort, calls, token_total, token_samples, cost_total, cost_samples, usage_time_ms, usage_time_samples, wall_time_ms, wall_time_samples, output_tokens_total, stream_duration_ms, output_speed_samples, first_byte_sum_ms, first_byte_samples, response_sum_ms, response_samples FROM active_daily WHERE dimension = ?1 AND stats_date BETWEEN ?2 AND ?3",
        long_term_projection_active_daily_cte(),
    );
    if series_key.is_some() {
        sql.push_str(" AND series_key = ?4");
    }
    sql.push_str(" ORDER BY stats_date ASC, series_key ASC");
    let mut query = sqlx::query_as::<_, LongTermRollupRow>(&sql)
        .bind(dimension)
        .bind(start_date)
        .bind(end_date);
    if let Some(series_key) = series_key {
        query = query.bind(series_key);
    }
    Ok(query.fetch_all(pool).await?)
}

pub(crate) async fn load_public_blog_token_activity_90d(
    pool: &Pool<Sqlite>,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Option<Vec<(String, Option<i64>)>>> {
    if (end_date - start_date).num_days() != 89 {
        bail!("public blog token activity requires a 90-day range");
    }
    let state = load_long_term_state(pool).await?;
    if state.status != LONG_TERM_STATUS_READY {
        return Ok(None);
    }
    let Some(covered_start) = state
        .statistics_start_date
        .as_deref()
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
    else {
        return Ok(None);
    };

    let query_start = start_date.max(covered_start);
    let rows = if query_start <= end_date {
        load_long_term_daily_rows(
            pool,
            "overall",
            None,
            &query_start.to_string(),
            &end_date.to_string(),
        )
        .await?
    } else {
        Vec::new()
    };
    let mut tokens_by_date = HashMap::<String, i64>::new();
    for row in rows {
        if row.token_total < 0 {
            bail!("long-term usage read model contains a negative token total");
        }
        let total = tokens_by_date.entry(row.bucket_or_date).or_default();
        *total = total
            .checked_add(row.token_total)
            .ok_or_else(|| anyhow!("long-term daily token total overflow"))?;
    }

    let mut points = Vec::with_capacity(90);
    let mut date = start_date;
    while date <= end_date {
        let key = date.to_string();
        let value = if date < query_start {
            None
        } else {
            tokens_by_date.get(&key).copied()
        };
        points.push((key, value));
        date = date
            .succ_opt()
            .ok_or_else(|| anyhow!("public blog token activity date overflow"))?;
    }
    Ok(Some(points))
}

pub(super) fn long_term_projection_active_daily_cte() -> &'static str {
    r#"
    WITH active_daily AS (
      SELECT d.stats_date, d.dimension, d.series_key, d.display_name, d.reasoning_effort,
        d.calls, d.token_total, d.token_samples, d.cost_total, d.cost_samples,
        d.usage_time_ms, d.usage_time_samples, d.wall_time_ms, d.wall_time_samples,
        d.output_tokens_total, d.stream_duration_ms, d.output_speed_samples,
        d.first_byte_sum_ms, d.first_byte_samples, d.response_sum_ms, d.response_samples
      FROM long_term_usage_daily d
      LEFT JOIN long_term_projection_bucket_state state ON state.bucket_date = d.stats_date
      LEFT JOIN long_term_projection_date_publications publication
        ON publication.publication_token = state.publication_token
      WHERE state.active_daily_backup_token IS NULL
         OR (
           publication.published = 1
           AND NOT EXISTS (
             SELECT 1
             FROM long_term_projection_dirty_buckets dirty
             WHERE dirty.bucket_date = d.stats_date
               AND (
                 state.publication_generation IS NULL
                 OR dirty.generation <> state.publication_generation
               )
           )
         )
      UNION ALL
      SELECT backup.stats_date, backup.dimension, backup.series_key, backup.display_name,
        backup.reasoning_effort, backup.calls, backup.token_total, backup.token_samples,
        backup.cost_total, backup.cost_samples, backup.usage_time_ms,
        backup.usage_time_samples, backup.wall_time_ms, backup.wall_time_samples,
        backup.output_tokens_total, backup.stream_duration_ms, backup.output_speed_samples,
        backup.first_byte_sum_ms, backup.first_byte_samples, backup.response_sum_ms,
        backup.response_samples
      FROM long_term_projection_daily_backups backup
      JOIN long_term_projection_bucket_state state
        ON state.bucket_date = backup.stats_date
       AND state.active_daily_backup_token = backup.rebuild_token
      LEFT JOIN long_term_projection_date_publications publication
        ON publication.publication_token = state.publication_token
      WHERE publication.published IS NULL
         OR publication.published = 0
         OR EXISTS (
           SELECT 1
           FROM long_term_projection_dirty_buckets dirty
           WHERE dirty.bucket_date = backup.stats_date
             AND (
               state.publication_generation IS NULL
               OR dirty.generation <> state.publication_generation
             )
         )
    )
    "#
}

pub(crate) async fn fetch_long_term_overview(
    State(state): State<Arc<AppState>>,
    Query(query): Query<LongTermRangeQuery>,
) -> Result<Json<LongTermStatsOverviewResponse>, (StatusCode, String)> {
    let range = LongTermRange::parse(query.range.as_deref())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid range".to_string()))?;
    let state_row = load_long_term_state(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let status = normalize_long_term_response_status(&state_row.status);
    if status != LONG_TERM_STATUS_READY {
        return Ok(Json(LongTermStatsOverviewResponse {
            status,
            statistics_start_date: state_row.statistics_start_date,
            processed_rows: state_row.processed_rows,
            total_rows: state_row.total_rows,
            timezone: LONG_TERM_TIMEZONE,
            range: range.as_str().to_string(),
            global: LongTermMetrics::default(),
            daily: Vec::new(),
            models: Vec::new(),
            upstreams: Vec::new(),
        }));
    }
    let (start_date, end_date) =
        long_term_date_window(range, state_row.statistics_start_date.as_deref());
    let daily_rows =
        load_long_term_daily_rows(&state.pool, "overall", None, &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?;
    let global = aggregate_rollup_rows(&daily_rows);
    let daily = build_daily_points(&daily_rows, &start_date, &end_date);
    let models = build_series_summaries(
        &load_long_term_rollups(&state.pool, range, "model", &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?,
    );
    let upstreams = build_series_summaries(
        &load_long_term_rollups(&state.pool, range, "upstream", &start_date, &end_date)
            .await
            .map_err(internal_error_tuple)?,
    );
    Ok(Json(LongTermStatsOverviewResponse {
        status,
        statistics_start_date: state_row.statistics_start_date,
        processed_rows: state_row.processed_rows,
        total_rows: state_row.total_rows,
        timezone: LONG_TERM_TIMEZONE,
        range: range.as_str().to_string(),
        global,
        daily,
        models,
        upstreams,
    }))
}

pub(crate) async fn fetch_long_term_series(
    State(state): State<Arc<AppState>>,
    OriginalUri(original_uri): OriginalUri,
) -> Result<Json<LongTermStatsSeriesResponse>, (StatusCode, String)> {
    let query = parse_long_term_series_query(&original_uri);
    let range = LongTermRange::parse(query.range.as_deref())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid range".to_string()))?;
    let dimension = query.dimension.as_deref().unwrap_or_default();
    if !matches!(dimension, "model" | "upstream") {
        return Err((StatusCode::BAD_REQUEST, "invalid dimension".to_string()));
    }
    let keys = query
        .key
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if keys.is_empty() || keys.len() > 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "key must contain 1 to 8 series keys".to_string(),
        ));
    }
    let state_row = load_long_term_state(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let status = normalize_long_term_response_status(&state_row.status);
    if status != LONG_TERM_STATUS_READY {
        return Ok(Json(LongTermStatsSeriesResponse {
            status,
            statistics_start_date: state_row.statistics_start_date,
            processed_rows: state_row.processed_rows,
            total_rows: state_row.total_rows,
            timezone: LONG_TERM_TIMEZONE,
            range: range.as_str().to_string(),
            dimension: dimension.to_string(),
            series: Vec::new(),
        }));
    }
    let (start_date, end_date) =
        long_term_date_window(range, state_row.statistics_start_date.as_deref());
    let available = load_long_term_rollups(&state.pool, range, dimension, &start_date, &end_date)
        .await
        .map_err(internal_error_tuple)?;
    let available_keys = available
        .iter()
        .map(|row| row.series_key.as_str())
        .collect::<HashSet<_>>();
    if keys
        .iter()
        .any(|key| !available_keys.contains(key.as_str()))
    {
        return Err((StatusCode::BAD_REQUEST, "unknown series key".to_string()));
    }
    let mut series = Vec::with_capacity(keys.len());
    for key in keys {
        let matching =
            load_long_term_daily_rows(&state.pool, dimension, Some(&key), &start_date, &end_date)
                .await
                .map_err(internal_error_tuple)?;
        let summary = available
            .iter()
            .find(|row| row.series_key.as_str() == key.as_str());
        series.push(LongTermSeries {
            series_key: key,
            display_name: summary
                .map(|row| row.display_name.clone())
                .unwrap_or_default(),
            reasoning_effort: summary.and_then(|row| {
                (!row.reasoning_effort.is_empty()).then_some(row.reasoning_effort.clone())
            }),
            points: build_daily_points(&matching, &start_date, &end_date),
        });
    }
    Ok(Json(LongTermStatsSeriesResponse {
        status,
        statistics_start_date: state_row.statistics_start_date,
        processed_rows: state_row.processed_rows,
        total_rows: state_row.total_rows,
        timezone: LONG_TERM_TIMEZONE,
        range: range.as_str().to_string(),
        dimension: dimension.to_string(),
        series,
    }))
}

pub(super) async fn load_long_term_state(pool: &Pool<Sqlite>) -> Result<LongTermStateRow> {
    Ok(sqlx::query_as::<_, LongTermStateRow>(
        "SELECT status, statistics_start_date, integrity_source_start_date, processed_rows, total_rows, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(pool)
    .await?)
}

pub(super) fn normalize_long_term_response_status(status: &str) -> String {
    match status {
        LONG_TERM_STATUS_READY | LONG_TERM_STATUS_EMPTY | LONG_TERM_STATUS_PREPARING => {
            status.to_string()
        }
        LONG_TERM_STATUS_DISABLED => LONG_TERM_STATUS_PREPARING.to_string(),
        LONG_TERM_STATUS_RUNNING => LONG_TERM_STATUS_PREPARING.to_string(),
        _ => LONG_TERM_STATUS_ERROR.to_string(),
    }
}

pub(super) fn long_term_date_window(
    range: LongTermRange,
    start_date: Option<&str>,
) -> (String, String) {
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let requested_start = today - ChronoDuration::days(range.days() - 1);
    let effective_start = start_date
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .map(|value| value.max(requested_start))
        .unwrap_or(requested_start);
    (effective_start.to_string(), today.to_string())
}

pub(super) fn build_series_summaries(rows: &[LongTermRollupRow]) -> Vec<LongTermSeriesSummary> {
    let mut summaries = rows
        .iter()
        .map(|row| LongTermSeriesSummary {
            series_key: row.series_key.clone(),
            display_name: row.display_name.clone(),
            reasoning_effort: (!row.reasoning_effort.is_empty())
                .then_some(row.reasoning_effort.clone()),
            metrics: LongTermMetrics::from_rollup(row),
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .metrics
            .tokens
            .unwrap_or_default()
            .cmp(&left.metrics.tokens.unwrap_or_default())
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    summaries
}

pub(super) fn aggregate_rollup_rows(rows: &[LongTermRollupRow]) -> LongTermMetrics {
    let mut acc = LongTermAccumulator::default();
    let mut wall_time_ms = 0.0;
    let mut wall_samples = 0_i64;
    for row in rows {
        acc.calls += row.calls;
        acc.token_total += row.token_total;
        acc.token_samples += row.token_samples;
        acc.cost_total += row.cost_total;
        acc.cost_samples += row.cost_samples;
        acc.usage_time_ms += row.usage_time_ms;
        acc.usage_time_samples += row.usage_time_samples;
        acc.output_tokens_total += row.output_tokens_total;
        acc.stream_duration_ms += row.stream_duration_ms;
        acc.output_speed_samples += row.output_speed_samples;
        acc.first_byte_sum_ms += row.first_byte_sum_ms;
        acc.first_byte_samples += row.first_byte_samples;
        acc.response_sum_ms += row.response_sum_ms;
        acc.response_samples += row.response_samples;
        wall_time_ms += row.wall_time_ms;
        wall_samples += row.wall_time_samples;
    }
    let mut metrics = LongTermMetrics::from_accumulator(&acc);
    metrics.wall_time_ms = (wall_samples > 0).then_some(wall_time_ms);
    metrics.wall_time_samples = wall_samples;
    metrics
}

pub(super) fn build_daily_points(
    rows: &[LongTermRollupRow],
    start_date: &str,
    end_date: &str,
) -> Vec<LongTermDailyPoint> {
    let mut by_date: HashMap<&str, (LongTermAccumulator, f64, i64)> = HashMap::new();
    for row in rows {
        let (acc, wall_time_ms, wall_samples) = by_date
            .entry(row.bucket_or_date.as_str())
            .or_insert_with(|| (LongTermAccumulator::default(), 0.0, 0));
        acc.calls += row.calls;
        acc.token_total += row.token_total;
        acc.token_samples += row.token_samples;
        acc.cost_total += row.cost_total;
        acc.cost_samples += row.cost_samples;
        acc.usage_time_ms += row.usage_time_ms;
        acc.usage_time_samples += row.usage_time_samples;
        acc.output_tokens_total += row.output_tokens_total;
        acc.stream_duration_ms += row.stream_duration_ms;
        acc.output_speed_samples += row.output_speed_samples;
        acc.first_byte_sum_ms += row.first_byte_sum_ms;
        acc.first_byte_samples += row.first_byte_samples;
        acc.response_sum_ms += row.response_sum_ms;
        acc.response_samples += row.response_samples;
        *wall_time_ms += row.wall_time_ms;
        *wall_samples += row.wall_time_samples;
    }
    let start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .unwrap_or_else(|_| Utc::now().with_timezone(&Shanghai).date_naive());
    let end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d").unwrap_or(start);
    let mut points = Vec::new();
    let mut date = start;
    while date <= end {
        let date_string = date.to_string();
        let metrics =
            if let Some((acc, wall_time_ms, wall_samples)) = by_date.get(date_string.as_str()) {
                let mut metrics = LongTermMetrics::from_accumulator(acc);
                metrics.wall_time_ms = (*wall_samples > 0).then_some(*wall_time_ms);
                metrics.wall_time_samples = *wall_samples;
                metrics
            } else {
                LongTermMetrics::default()
            };
        points.push(LongTermDailyPoint {
            date: date_string,
            metrics,
        });
        let Some(next) = date.succ_opt() else { break };
        date = next;
    }
    points
}

pub(super) fn normalize_long_term_model(row: &LongTermInvocationRow) -> String {
    for candidate in [
        row.response_model.as_deref(),
        row.model.as_deref(),
        row.request_model.as_deref(),
    ] {
        if let Some(value) = candidate.map(str::trim).filter(|value| !value.is_empty()) {
            return value.to_string();
        }
    }
    "未知模型".to_string()
}

pub(super) fn normalize_long_term_reasoning(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未指定")
        .to_string()
}

pub(super) fn long_term_model_series_key(model: &str, reasoning: &str) -> String {
    let payload =
        serde_json::to_vec(&[model, reasoning]).expect("model series key payload is serializable");
    format!(
        "model:v2:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    )
}

pub(super) fn normalize_long_term_upstream(row: &LongTermInvocationRow) -> (String, String) {
    if row
        .upstream_account_kind
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX))
        && let Some(id) = row.upstream_account_id
    {
        return (
            format!("account:{id}"),
            row.upstream_account_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("账号 {id}")),
        );
    }
    (
        LONG_TERM_OTHER_KEY.to_string(),
        LONG_TERM_OTHER_NAME.to_string(),
    )
}

pub(super) fn is_success_status(status: Option<&str>, error_message: Option<&str>) -> bool {
    invocation_status_is_success_like(status, error_message)
        || matches!(
            status
                .map(str::trim)
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "succeeded" | "ok"
        )
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LongTermTimestamp {
    pub(super) epoch_ms: i64,
    pub(super) sub_millisecond_nanos: u32,
}

pub(super) fn parse_long_term_timestamp(raw: &str) -> Option<LongTermTimestamp> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(LongTermTimestamp {
            epoch_ms: value.timestamp_millis(),
            sub_millisecond_nanos: value.timestamp_subsec_nanos() % 1_000_000,
        });
    }
    let naive = NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    Shanghai
        .from_local_datetime(&naive)
        .single()
        .map(|value| LongTermTimestamp {
            epoch_ms: value.timestamp_millis(),
            sub_millisecond_nanos: value.timestamp_subsec_nanos() % 1_000_000,
        })
}

pub(super) fn parse_long_term_timestamp_ms(raw: &str) -> Option<i64> {
    parse_long_term_timestamp(raw).map(|timestamp| timestamp.epoch_ms)
}

pub(super) fn long_term_interval_end_ms(start: LongTermTimestamp, duration_ms: f64) -> Option<i64> {
    let elapsed_nanos = duration_ms * 1_000_000.0;
    if !elapsed_nanos.is_finite() || elapsed_nanos <= 0.0 {
        return None;
    }
    // Interval state is stored in milliseconds. Round its exclusive end up so a positive
    // sub-millisecond tail that crosses a date or hour boundary remains materialized there.
    let elapsed_millis = ((start.sub_millisecond_nanos as f64 + elapsed_nanos) / 1_000_000.0)
        .ceil()
        .clamp(1.0, i64::MAX as f64) as i64;
    start.epoch_ms.checked_add(elapsed_millis)
}

pub(super) fn union_interval_duration(intervals: &[(i64, i64)]) -> i64 {
    if intervals.is_empty() {
        return 0;
    }
    let mut sorted = intervals.to_vec();
    sorted.sort_unstable_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut total = 0_i64;
    let mut current = sorted[0];
    for next in sorted.into_iter().skip(1) {
        if next.0 <= current.1 {
            current.1 = current.1.max(next.1);
        } else {
            total += current.1.saturating_sub(current.0);
            current = next;
        }
    }
    total + current.1.saturating_sub(current.0)
}

pub(super) fn internal_error_tuple(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}
