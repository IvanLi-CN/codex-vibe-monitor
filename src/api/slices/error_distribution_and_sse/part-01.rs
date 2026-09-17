pub(crate) fn align_reporting_bucket_epoch(
    epoch: i64,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<i64> {
    let timestamp = Utc
        .timestamp_opt(epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
    let local = timestamp.with_timezone(&reporting_tz);
    let elapsed_seconds = i64::from(local.time().num_seconds_from_midnight());
    let remainder = elapsed_seconds.rem_euclid(bucket_seconds);
    let bucket_start_local = local.naive_local() - ChronoDuration::seconds(remainder);
    Ok(
        local_naive_to_utc_not_after_reference(bucket_start_local, reporting_tz, timestamp)
            .timestamp(),
    )
}

pub(crate) fn next_reporting_bucket_epoch(
    bucket_start_epoch: i64,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<i64> {
    let bucket_start = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid bucket epoch"))?;
    let next_start = if bucket_seconds == 3_600 {
        bucket_start + ChronoDuration::seconds(bucket_seconds)
    } else {
        let local_start = bucket_start.with_timezone(&reporting_tz).naive_local();
        local_naive_to_utc(
            local_start + ChronoDuration::seconds(bucket_seconds),
            reporting_tz,
        )
    };
    if next_start.timestamp() <= bucket_start_epoch {
        return Err(anyhow!(
            "non-increasing reporting bucket progression for {reporting_tz} at {bucket_start_epoch}"
        ));
    }
    Ok(next_start.timestamp())
}

pub(crate) fn resolve_complete_parallel_work_window(
    now: DateTime<Utc>,
    duration: ChronoDuration,
    bucket_seconds: i64,
    reporting_tz: Tz,
) -> Result<RangeWindow> {
    let end_epoch = align_reporting_bucket_epoch(now.timestamp(), bucket_seconds, reporting_tz)?;
    let end = Utc
        .timestamp_opt(end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work window end epoch"))?;
    let start = local_naive_to_utc(
        end.with_timezone(&reporting_tz).naive_local() - duration,
        reporting_tz,
    );
    Ok(RangeWindow {
        start,
        end,
        display_end: end,
        duration,
    })
}

pub(crate) fn resolve_parallel_work_rollup_reporting_tz(
    requested_reporting_tz: Tz,
    range_window: &RangeWindow,
) -> (Tz, bool) {
    if reporting_tz_has_whole_hour_offsets(requested_reporting_tz, range_window) {
        return (requested_reporting_tz, false);
    }
    (Shanghai, true)
}

pub(crate) struct ParallelWorkWindowResponseInput {
    pub(crate) range_start: DateTime<Utc>,
    pub(crate) range_end: DateTime<Utc>,
    pub(crate) bucket_seconds: i64,
    pub(crate) reporting_tz: Tz,
    pub(crate) counts_by_bucket: BTreeMap<i64, i64>,
    pub(crate) active_minute_stats: ParallelWorkActiveMinuteStats,
    pub(crate) effective_time_zone: Tz,
    pub(crate) time_zone_fallback: bool,
    pub(crate) conversations: Vec<ParallelWorkConversation>,
}

pub(crate) fn build_parallel_work_window_response(
    input: ParallelWorkWindowResponseInput,
) -> Result<ParallelWorkWindowResponse> {
    let ParallelWorkWindowResponseInput {
        range_start,
        range_end,
        bucket_seconds,
        reporting_tz,
        counts_by_bucket,
        active_minute_stats,
        effective_time_zone,
        time_zone_fallback,
        conversations,
    } = input;
    if range_start >= range_end {
        return Ok(empty_parallel_work_window_response(
            range_end,
            bucket_seconds,
            effective_time_zone,
            time_zone_fallback,
        ));
    }

    let mut points = Vec::new();
    let mut cursor = range_start.timestamp();
    let end_epoch = range_end.timestamp();
    let mut min_count: Option<i64> = None;
    let mut max_count: Option<i64> = None;
    let mut active_bucket_count = 0_i64;

    while cursor < end_epoch {
        let next = next_reporting_bucket_epoch(cursor, bucket_seconds, reporting_tz)?;
        if next > end_epoch {
            break;
        }
        let parallel_count = counts_by_bucket.get(&cursor).copied().unwrap_or_default();
        if parallel_count > 0 {
            active_bucket_count += 1;
        }
        min_count = Some(match min_count {
            Some(current) => current.min(parallel_count),
            None => parallel_count,
        });
        max_count = Some(match max_count {
            Some(current) => current.max(parallel_count),
            None => parallel_count,
        });
        points.push(ParallelWorkPoint {
            bucket_start: format_utc_iso(
                Utc.timestamp_opt(cursor, 0)
                    .single()
                    .ok_or_else(|| anyhow!("invalid parallel-work bucket start epoch"))?,
            ),
            bucket_end: format_utc_iso(
                Utc.timestamp_opt(next, 0)
                    .single()
                    .ok_or_else(|| anyhow!("invalid parallel-work bucket end epoch"))?,
            ),
            parallel_count,
        });
        cursor = next;
    }

    let complete_bucket_count = points.len() as i64;
    Ok(ParallelWorkWindowResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        bucket_seconds,
        complete_bucket_count,
        active_bucket_count,
        active_minute_count: active_minute_stats.active_minute_count,
        min_count,
        max_count,
        avg_count: active_minute_stats.average(),
        effective_time_zone: effective_time_zone.to_string(),
        time_zone_fallback,
        points,
        conversations,
    })
}

pub(crate) fn empty_parallel_work_window_response(
    boundary: DateTime<Utc>,
    bucket_seconds: i64,
    effective_time_zone: Tz,
    time_zone_fallback: bool,
) -> ParallelWorkWindowResponse {
    ParallelWorkWindowResponse {
        range_start: format_utc_iso(boundary),
        range_end: format_utc_iso(boundary),
        bucket_seconds,
        complete_bucket_count: 0,
        active_bucket_count: 0,
        active_minute_count: None,
        min_count: None,
        max_count: None,
        avg_count: None,
        effective_time_zone: effective_time_zone.to_string(),
        time_zone_fallback,
        points: Vec::new(),
        conversations: Vec::new(),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ParallelWorkActiveMinuteStats {
    pub(crate) active_minute_count: Option<i64>,
    pub(crate) parallel_count_sum: i64,
}

impl ParallelWorkActiveMinuteStats {
    pub(crate) fn unavailable() -> Self {
        Self::default()
    }

    pub(crate) fn empty_available() -> Self {
        Self {
            active_minute_count: Some(0),
            parallel_count_sum: 0,
        }
    }

    pub(crate) fn from_key_sets(bucket_keys: BTreeMap<i64, HashSet<String>>) -> Self {
        let active_minute_count = bucket_keys.len() as i64;
        let parallel_count_sum = bucket_keys
            .values()
            .map(|prompt_cache_keys| prompt_cache_keys.len() as i64)
            .sum();
        Self {
            active_minute_count: Some(active_minute_count),
            parallel_count_sum,
        }
    }

    pub(crate) fn combine(self, other: Self) -> Self {
        match (self.active_minute_count, other.active_minute_count) {
            (Some(left), Some(right)) => Self {
                active_minute_count: Some(left + right),
                parallel_count_sum: self.parallel_count_sum + other.parallel_count_sum,
            },
            _ => Self::unavailable(),
        }
    }

    pub(crate) fn average(self) -> Option<f64> {
        self.active_minute_count
            .filter(|active_minute_count| *active_minute_count > 0)
            .map(|active_minute_count| self.parallel_count_sum as f64 / active_minute_count as f64)
    }
}

pub(crate) fn parallel_work_counts_from_key_sets(
    bucket_keys: BTreeMap<i64, HashSet<String>>,
) -> BTreeMap<i64, i64> {
    bucket_keys
        .into_iter()
        .map(|(bucket_start_epoch, prompt_cache_keys)| {
            (bucket_start_epoch, prompt_cache_keys.len() as i64)
        })
        .collect()
}

fn first_complete_minute_epoch(value: DateTime<Utc>) -> i64 {
    let epoch = value.timestamp();
    if epoch.rem_euclid(60) == 0 {
        epoch
    } else {
        epoch.div_euclid(60) * 60 + 60
    }
}

fn end_of_complete_minutes_epoch(value: DateTime<Utc>) -> i64 {
    value.timestamp().div_euclid(60) * 60
}

async fn parallel_work_hourly_coverage_is_complete(
    pool: &Pool<Sqlite>,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: InvocationSourceScope,
    field: &str,
) -> Result<bool> {
    if start_epoch >= end_epoch {
        return Ok(true);
    }
    let expected_hour_count = (end_epoch - start_epoch) / 3_600;
    let field = match field {
        "minute_keys_complete" => "minute_keys_complete",
        "hourly_scalar_complete" => "hourly_scalar_complete",
        _ => return Err(anyhow!("invalid parallel-work coverage field")),
    };
    let count: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM parallel_work_hourly_coverage \
         WHERE hour_start_epoch >= ?1 AND hour_start_epoch < ?2 \
           AND source_scope = ?3 AND {field} = 1"
    ))
    .bind(start_epoch)
    .bind(end_epoch)
    .bind(parallel_work_source_scope_name(source_scope))
    .fetch_one(pool)
    .await?;
    Ok(count == expected_hour_count)
}

async fn query_parallel_work_minute_rollup_key_sets(
    pool: &Pool<Sqlite>,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<BTreeMap<i64, HashSet<String>>> {
    let table = if upstream_account_id.is_some() {
        "parallel_work_upstream_account_minute_key_rollup"
    } else {
        "parallel_work_minute_key_rollup"
    };
    let mut query = QueryBuilder::<Sqlite>::new(format!(
        "SELECT minute_start_epoch AS bucket_start_epoch, prompt_cache_key FROM {table} WHERE minute_start_epoch >= "
    ));
    query
        .push_bind(start_epoch)
        .push(" AND minute_start_epoch < ")
        .push_bind(end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    query.push(" ORDER BY minute_start_epoch ASC, prompt_cache_key ASC");
    let rows = query
        .build_query_as::<ParallelWorkDayRollupRow>()
        .fetch_all(pool)
        .await?;
    let mut bucket_keys = BTreeMap::<i64, HashSet<String>>::new();
    for row in rows {
        bucket_keys
            .entry(row.bucket_start_epoch)
            .or_default()
            .insert(row.prompt_cache_key);
    }
    Ok(bucket_keys)
}

async fn query_parallel_work_hourly_scalar_stats(
    pool: &Pool<Sqlite>,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats> {
    #[derive(FromRow)]
    struct ScalarSums {
        active_minute_count: Option<i64>,
        parallel_count_sum: Option<i64>,
    }

    let source_scope = parallel_work_source_scope_name(source_scope);
    let row = if let Some(upstream_account_id) = upstream_account_id {
        sqlx::query_as::<_, ScalarSums>(
            r#"
            SELECT SUM(active_minute_count) AS active_minute_count,
                   SUM(parallel_count_sum) AS parallel_count_sum
            FROM parallel_work_upstream_account_hourly_rollup
            WHERE hour_start_epoch >= ?1 AND hour_start_epoch < ?2
              AND source_scope = ?3 AND upstream_account_id = ?4
            "#,
        )
        .bind(start_epoch)
        .bind(end_epoch)
        .bind(source_scope)
        .bind(upstream_account_id)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_as::<_, ScalarSums>(
            r#"
            SELECT SUM(active_minute_count) AS active_minute_count,
                   SUM(parallel_count_sum) AS parallel_count_sum
            FROM parallel_work_hourly_rollup
            WHERE hour_start_epoch >= ?1 AND hour_start_epoch < ?2
              AND source_scope = ?3
            "#,
        )
        .bind(start_epoch)
        .bind(end_epoch)
        .bind(source_scope)
        .fetch_one(pool)
        .await?
    };
    Ok(ParallelWorkActiveMinuteStats {
        active_minute_count: Some(row.active_minute_count.unwrap_or_default()),
        parallel_count_sum: row.parallel_count_sum.unwrap_or_default(),
    })
}

pub(crate) async fn query_parallel_work_active_minute_stats(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    raw_detail_start_epoch: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats> {
    let complete_start_epoch = first_complete_minute_epoch(range_start);
    let complete_end_epoch = end_of_complete_minutes_epoch(range_end);
    if complete_start_epoch >= complete_end_epoch {
        return Ok(ParallelWorkActiveMinuteStats::empty_available());
    }
    query_parallel_work_active_minute_stats_for_bounds(
        pool,
        complete_start_epoch,
        complete_end_epoch,
        source_scope,
        upstream_account_id,
        raw_detail_start_epoch,
    )
    .await
}

async fn query_parallel_work_active_minute_stats_for_bounds(
    pool: &Pool<Sqlite>,
    complete_start_epoch: i64,
    complete_end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    raw_detail_start_epoch: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats> {
    let complete_end = Utc
        .timestamp_opt(complete_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work complete-minute end"))?;
    let minute_keep_start_epoch = parallel_work_minute_rollup_keep_start_epoch(Utc::now())?;
    let current_hour_start_epoch = Utc::now().timestamp().div_euclid(3_600) * 3_600;
    let scalar_end_epoch = complete_end_epoch.min(minute_keep_start_epoch);
    let mut result = if complete_start_epoch < scalar_end_epoch {
        query_parallel_work_scalar_stats(
            pool,
            complete_start_epoch,
            scalar_end_epoch,
            source_scope,
            upstream_account_id,
        )
        .await?
    } else {
        ParallelWorkActiveMinuteStats::empty_available()
    };

    let minute_start_epoch = complete_start_epoch.max(minute_keep_start_epoch);
    let minute_end_epoch = complete_end_epoch.min(current_hour_start_epoch);
    if minute_start_epoch < minute_end_epoch {
        let minute_stats = query_parallel_work_minute_stats(
            pool,
            minute_start_epoch,
            minute_end_epoch,
            source_scope,
            upstream_account_id,
            raw_detail_start_epoch,
        )
        .await?;
        result = result.combine(minute_stats);
    }

    let raw_tail_start_epoch = complete_start_epoch.max(current_hour_start_epoch);
    if raw_tail_start_epoch < complete_end_epoch {
        result = result.combine(
            query_parallel_work_raw_tail_stats(
                pool,
                raw_tail_start_epoch,
                complete_end,
                source_scope,
                upstream_account_id,
            )
            .await?,
        );
    }
    Ok(result)
}

async fn query_parallel_work_scalar_stats(
    pool: &Pool<Sqlite>,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats> {
    if start_epoch.rem_euclid(3_600) != 0
        || end_epoch.rem_euclid(3_600) != 0
        || !parallel_work_hourly_coverage_is_complete(
            pool,
            start_epoch,
            end_epoch,
            source_scope,
            "hourly_scalar_complete",
        )
        .await?
    {
        return Ok(ParallelWorkActiveMinuteStats::unavailable());
    }
    query_parallel_work_hourly_scalar_stats(
        pool,
        start_epoch,
        end_epoch,
        source_scope,
        upstream_account_id,
    )
    .await
}

async fn query_parallel_work_minute_stats(
    pool: &Pool<Sqlite>,
    start_epoch: i64,
    end_epoch: i64,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    raw_detail_start_epoch: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats> {
    let coverage_start_epoch = start_epoch.div_euclid(3_600) * 3_600;
    let coverage_end_epoch = (end_epoch.saturating_add(3_599)).div_euclid(3_600) * 3_600;
    if parallel_work_hourly_coverage_is_complete(
        pool,
        coverage_start_epoch,
        coverage_end_epoch,
        source_scope,
        "minute_keys_complete",
    )
    .await?
    {
        return Ok(ParallelWorkActiveMinuteStats::from_key_sets(
            query_parallel_work_minute_rollup_key_sets(
                pool,
                start_epoch,
                end_epoch,
                source_scope,
                upstream_account_id,
            )
            .await?,
        ));
    }
    if raw_detail_start_epoch.is_none_or(|start| start > start_epoch) {
        return Ok(ParallelWorkActiveMinuteStats::unavailable());
    }
    let minute_start = Utc
        .timestamp_opt(start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work minute start"))?;
    let minute_end = Utc
        .timestamp_opt(end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work minute end"))?;
    Ok(ParallelWorkActiveMinuteStats::from_key_sets(
        query_parallel_work_exact_key_sets(
            pool,
            ParallelWorkExactKeySetsQuery {
                range_start: minute_start,
                range_end: minute_end,
                bucket_seconds: 60,
                reporting_tz: chrono_tz::UTC,
                source_scope,
                upstream_account_id,
                start_after_id: None,
                snapshot_id: None,
            },
        )
        .await?,
    ))
}

async fn query_parallel_work_raw_tail_stats(
    pool: &Pool<Sqlite>,
    start_epoch: i64,
    end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<ParallelWorkActiveMinuteStats> {
    let start = Utc
        .timestamp_opt(start_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work raw-tail start"))?;
    Ok(ParallelWorkActiveMinuteStats::from_key_sets(
        query_parallel_work_exact_key_sets(
            pool,
            ParallelWorkExactKeySetsQuery {
                range_start: start,
                range_end: end,
                bucket_seconds: 60,
                reporting_tz: chrono_tz::UTC,
                source_scope,
                upstream_account_id,
                start_after_id: None,
                snapshot_id: None,
            },
        )
        .await?,
    ))
}

pub(crate) struct ParallelWorkExactKeySetsQuery {
    pub(crate) range_start: DateTime<Utc>,
    pub(crate) range_end: DateTime<Utc>,
    pub(crate) bucket_seconds: i64,
    pub(crate) reporting_tz: Tz,
    pub(crate) source_scope: InvocationSourceScope,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) start_after_id: Option<i64>,
    pub(crate) snapshot_id: Option<i64>,
}

pub(crate) async fn query_parallel_work_exact_key_sets(
    pool: &Pool<Sqlite>,
    input: ParallelWorkExactKeySetsQuery,
) -> Result<BTreeMap<i64, HashSet<String>>> {
    let ParallelWorkExactKeySetsQuery {
        range_start,
        range_end,
        bucket_seconds,
        reporting_tz,
        source_scope,
        upstream_account_id,
        start_after_id,
        snapshot_id,
    } = input;
    let mut query = QueryBuilder::new("SELECT occurred_at, ");
    query
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" AS prompt_cache_key FROM codex_invocations WHERE occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_start))
        .push(" AND occurred_at < ")
        .push_bind(db_occurred_at_lower_bound(range_end))
        .push(" AND ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" IS NOT NULL AND ")
        .push(INVOCATION_PROMPT_CACHE_KEY_SQL)
        .push(" != ''");
    if let Some(start_after_id) = start_after_id {
        query.push(" AND id > ").push_bind(start_after_id);
    }
    if let Some(snapshot_id) = snapshot_id {
        query.push(" AND id <= ").push_bind(snapshot_id);
    }
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND ")
            .push(
                crate::api::invocation_upstream_account_id_with_attempt_fallback_sql(
                    "codex_invocations",
                ),
            )
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    query.push(" ORDER BY occurred_at ASC, id ASC, prompt_cache_key ASC");

    let rows = query
        .build_query_as::<ParallelWorkExactInvocationRow>()
        .fetch_all(pool)
        .await?;
    let mut bucket_keys: BTreeMap<i64, HashSet<String>> = BTreeMap::new();
    for row in rows {
        let Some(occurred_at) = parse_to_utc_datetime(&row.occurred_at) else {
            continue;
        };
        let bucket_start_epoch =
            align_reporting_bucket_epoch(occurred_at.timestamp(), bucket_seconds, reporting_tz)?;
        bucket_keys
            .entry(bucket_start_epoch)
            .or_default()
            .insert(row.prompt_cache_key);
    }
    Ok(bucket_keys)
}

pub(crate) async fn query_parallel_work_bucket_key_sets_from_hourly_rollups(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
) -> Result<BTreeMap<i64, HashSet<String>>> {
    let mut query = if upstream_account_id.is_some() {
        QueryBuilder::new(
            "SELECT bucket_start_epoch, prompt_cache_key FROM prompt_cache_upstream_account_hourly \
             WHERE bucket_start_epoch >= ",
        )
    } else {
        QueryBuilder::new(
            "SELECT bucket_start_epoch, prompt_cache_key FROM prompt_cache_rollup_hourly \
             WHERE bucket_start_epoch >= ",
        )
    };
    query
        .push_bind(range_start.timestamp())
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end.timestamp())
        .push(" AND prompt_cache_key != ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    if let Some(upstream_account_id) = upstream_account_id {
        query
            .push(" AND upstream_account_id = ")
            .push_bind(upstream_account_id);
    }
    query.push(" ORDER BY bucket_start_epoch ASC, prompt_cache_key ASC");

    let mut rows = query
        .build_query_as::<ParallelWorkDayRollupRow>()
        .fetch(pool);
    let mut bucket_keys: BTreeMap<i64, HashSet<String>> = BTreeMap::new();

    while let Some(row) = rows.try_next().await? {
        let bucket_epoch =
            align_reporting_bucket_epoch(row.bucket_start_epoch, bucket_seconds, reporting_tz)?;
        bucket_keys
            .entry(bucket_epoch)
            .or_default()
            .insert(row.prompt_cache_key);
    }

    Ok(bucket_keys)
}

pub(crate) fn should_fallback_parallel_work_day_all_window(
    requested_reporting_tz: Tz,
    requested_window: Option<&RangeWindow>,
    now: DateTime<Utc>,
) -> bool {
    if let Some(window) = requested_window {
        return !reporting_tz_has_whole_hour_offsets(requested_reporting_tz, window);
    }

    let latest_complete_day_end = local_midnight_utc(
        now.with_timezone(&requested_reporting_tz).date_naive(),
        requested_reporting_tz,
    );
    let probe_start = latest_complete_day_end - ChronoDuration::days(1);
    let probe_window = RangeWindow {
        start: probe_start,
        end: latest_complete_day_end,
        display_end: latest_complete_day_end,
        duration: ChronoDuration::days(1),
    };
    !reporting_tz_has_whole_hour_offsets(requested_reporting_tz, &probe_window)
}

pub(crate) fn local_naive_to_utc_not_after_reference(
    naive: NaiveDateTime,
    tz: Tz,
    reference_utc: DateTime<Utc>,
) -> DateTime<Utc> {
    match tz.from_local_datetime(&naive) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(first, second) => {
            let first_utc = first.with_timezone(&Utc);
            let second_utc = second.with_timezone(&Utc);
            [first_utc, second_utc]
                .into_iter()
                .filter(|candidate| *candidate <= reference_utc)
                .max()
                .unwrap_or(first_utc.min(second_utc))
        }
        LocalResult::None => local_naive_to_utc(naive, tz),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailureScope {
    All,
    Service,
    Client,
    Abort,
}

impl FailureScope {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self, ApiError> {
        let Some(scope) = raw.map(str::trim).filter(|v| !v.is_empty()) else {
            return Ok(FailureScope::Service);
        };
        match scope.to_ascii_lowercase().as_str() {
            "all" => Ok(FailureScope::All),
            "service" => Ok(FailureScope::Service),
            "client" => Ok(FailureScope::Client),
            "abort" => Ok(FailureScope::Abort),
            _ => Err(ApiError::bad_request(anyhow!(
                "unsupported failure scope: {scope}; expected one of all|service|client|abort"
            ))),
        }
    }
}

pub(crate) fn failure_scope_matches(scope: FailureScope, class: FailureClass) -> bool {
    match scope {
        FailureScope::All => class != FailureClass::None,
        FailureScope::Service => class == FailureClass::ServiceFailure,
        FailureScope::Client => class == FailureClass::ClientFailure,
        FailureScope::Abort => class == FailureClass::ClientAbort,
    }
}

pub(crate) fn extract_failure_kind_prefix(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    let closing = trimmed.find(']')?;
    if closing <= 1 {
        return None;
    }
    Some(trimmed[1..closing].trim().to_string())
}

pub(crate) fn derive_failure_kind(status_norm: &str, err: &str, err_lower: &str) -> Option<String> {
    if err_lower.contains("downstream closed while streaming upstream response") {
        return Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED.to_string());
    }
    if err_lower.contains("upstream response stream reported failure") {
        return Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string());
    }
    if err_lower.contains("upstream stream error") {
        return Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR.to_string());
    }
    if err_lower.contains("failed to contact upstream") {
        return Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    }
    if err_lower.contains("[upstream_response_failed]")
        || err_lower.contains("upstream response failed")
    {
        return Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED.to_string());
    }
    if err_lower.contains("upstream handshake timed out") {
        return Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT.to_string());
    }
    if err_lower.contains("request body read timed out") {
        return Some(PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT.to_string());
    }
    if err_lower.contains("failed to read request body stream") {
        return Some(PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED.to_string());
    }
    if err_lower.contains("invalid api key format")
        || err_lower.contains("api key format is invalid")
        || err_lower.contains("incorrect api key provided")
    {
        return Some("invalid_api_key".to_string());
    }
    if err_lower.contains("api key not found") {
        return Some("api_key_not_found".to_string());
    }
    if err_lower.contains("please provide an api key") {
        return Some("api_key_missing".to_string());
    }
    if status_norm == "http_200" && err.is_empty() {
        return None;
    }
    if status_norm.starts_with("http_") {
        return Some(status_norm.to_string());
    }
    if !err.is_empty() {
        return Some("untyped_failure".to_string());
    }
    None
}
