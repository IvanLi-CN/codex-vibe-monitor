#[expect(
    clippy::too_many_arguments,
    reason = "The bucket key and invocation timing are kept explicit at this aggregation boundary."
)]
fn add_long_term_row(
    buckets: &mut HashMap<(i64, String, String), LongTermBucket>,
    dimension: &str,
    series_key: &str,
    display_name: &str,
    reasoning_effort: &str,
    start_ms: i64,
    row: &LongTermInvocationRow,
    interval: Option<(i64, i64)>,
) {
    let hour_start_ms = start_ms.div_euclid(LONG_TERM_HOUR_MS) * LONG_TERM_HOUR_MS;
    let original_interval = interval;
    let interval = interval.and_then(|(start, end)| {
        let hour_end = hour_start_ms + LONG_TERM_HOUR_MS;
        let clipped_start = start.max(hour_start_ms);
        let clipped_end = end.min(hour_end);
        (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
    });
    let key = (
        hour_start_ms / 1000,
        dimension.to_string(),
        series_key.to_string(),
    );
    let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
        bucket_start_epoch: hour_start_ms / 1000,
        dimension: dimension.to_string(),
        series_key: series_key.to_string(),
        display_name: display_name.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        stats_date: None,
        accumulator: LongTermAccumulator::default(),
    });
    bucket.accumulator.add_call(row, interval);
    let Some((interval_start, interval_end)) = original_interval else {
        return;
    };
    let mut next_hour_start = hour_start_ms + LONG_TERM_HOUR_MS;
    while next_hour_start < interval_end {
        let segment_start = next_hour_start.max(interval_start);
        let segment_end = (next_hour_start + LONG_TERM_HOUR_MS).min(interval_end);
        if segment_end > segment_start {
            let key = (
                next_hour_start / 1000,
                dimension.to_string(),
                series_key.to_string(),
            );
            let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
                bucket_start_epoch: next_hour_start / 1000,
                dimension: dimension.to_string(),
                series_key: series_key.to_string(),
                display_name: display_name.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
                stats_date: None,
                accumulator: LongTermAccumulator::default(),
            });
            bucket
                .accumulator
                .add_interval(Some((segment_start, segment_end)));
        }
        next_hour_start += LONG_TERM_HOUR_MS;
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "The bucket key and invocation timing are kept explicit at this aggregation boundary."
)]
fn add_long_term_daily_row(
    buckets: &mut HashMap<(String, String, String), LongTermBucket>,
    dimension: &str,
    series_key: &str,
    display_name: &str,
    reasoning_effort: &str,
    date: &str,
    row: &LongTermInvocationRow,
    interval: Option<(i64, i64)>,
) {
    let original_interval = interval;
    let key = (
        date.to_string(),
        dimension.to_string(),
        series_key.to_string(),
    );
    let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
        bucket_start_epoch: 0,
        dimension: dimension.to_string(),
        series_key: series_key.to_string(),
        display_name: display_name.to_string(),
        reasoning_effort: reasoning_effort.to_string(),
        stats_date: Some(date.to_string()),
        accumulator: LongTermAccumulator::default(),
    });
    let interval = interval.and_then(|(start, end)| {
        let day_start = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()?
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())?;
        let day_end = day_start.checked_add_signed(ChronoDuration::days(1))?;
        let clipped_start = start.max(day_start.timestamp_millis());
        let clipped_end = end.min(day_end.timestamp_millis());
        (clipped_end > clipped_start).then_some((clipped_start, clipped_end))
    });
    bucket.accumulator.add_call(row, interval);
    let Some((interval_start, interval_end)) = original_interval else {
        return;
    };
    let Some(mut next_date) = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|value| value.succ_opt())
    else {
        return;
    };
    loop {
        let Some(day_start) = next_date
            .and_hms_opt(0, 0, 0)
            .and_then(|value| Shanghai.from_local_datetime(&value).single())
        else {
            return;
        };
        if day_start.timestamp_millis() >= interval_end {
            break;
        }
        let day_end = day_start
            .checked_add_signed(ChronoDuration::days(1))
            .map(|value| value.timestamp_millis())
            .unwrap_or(interval_end);
        let segment_start = interval_start.max(day_start.timestamp_millis());
        let segment_end = interval_end.min(day_end);
        if segment_end > segment_start {
            let date_string = next_date.to_string();
            let key = (
                date_string.clone(),
                dimension.to_string(),
                series_key.to_string(),
            );
            let bucket = buckets.entry(key).or_insert_with(|| LongTermBucket {
                bucket_start_epoch: 0,
                dimension: dimension.to_string(),
                series_key: series_key.to_string(),
                display_name: display_name.to_string(),
                reasoning_effort: reasoning_effort.to_string(),
                stats_date: Some(date_string),
                accumulator: LongTermAccumulator::default(),
            });
            bucket
                .accumulator
                .add_interval(Some((segment_start, segment_end)));
        }
        if segment_end >= interval_end {
            break;
        }
        let Some(next) = next_date.succ_opt() else {
            break;
        };
        next_date = next;
    }
}

fn projection_interval_key(
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

fn collect_long_term_projection_interval_segments(
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

fn merge_long_term_projection_bucket(target: &mut LongTermBucket, source: &LongTermBucket) {
    target.accumulator.merge(&source.accumulator);
}

fn merge_long_term_projection_buckets<K>(
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

async fn insert_long_term_hourly(
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

async fn insert_long_term_daily(
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

async fn insert_long_term_rollup(
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

async fn load_long_term_projection_interval_index(
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
    let canonical_rows = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
        "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE interval_start_ms < ?1 AND interval_end_ms > ?2",
    )
    .bind(last_end_ms)
    .bind(first_start_ms)
    .fetch_all(pool)
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

fn add_long_term_projection_interval_state_for_date(
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

fn long_term_projection_interval_dates(
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

async fn upsert_long_term_projection_interval_segments(
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

async fn upsert_long_term_projection_interval_segments_in_transaction(
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

async fn load_long_term_projection_interval_state_ids(
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

async fn migrate_long_term_projection_legacy_interval_state(
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

async fn merge_long_term_projection_rollup(
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

async fn apply_long_term_projection_incremental(
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

async fn apply_long_term_projection_incremental_with_runtime(
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

struct LongTermProjectionIncrementalBatch<'a> {
    hourly: &'a HashMap<(i64, String, String), LongTermBucket>,
    daily: &'a HashMap<(String, String, String), LongTermBucket>,
    segments: &'a [LongTermProjectionIntervalSegment],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LongTermProjectionIncrementalOutcome {
    Published,
    RebuildRequired,
}

async fn long_term_projection_incremental_dates_are_dirty(
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
