use super::*;
use anyhow::anyhow;
use chrono::Timelike;
use serde::Serialize;
use sqlx::FromRow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::warn;

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

pub(crate) fn build_parallel_work_window_response(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    counts_by_bucket: &BTreeMap<i64, i64>,
    active_minute_stats: ParallelWorkActiveMinuteStats,
    effective_time_zone: Tz,
    time_zone_fallback: bool,
    conversations: Vec<ParallelWorkConversation>,
) -> Result<ParallelWorkWindowResponse> {
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
    let complete_end = Utc
        .timestamp_opt(complete_end_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid parallel-work complete-minute end"))?;
    let minute_keep_start_epoch = parallel_work_minute_rollup_keep_start_epoch(Utc::now())?;
    let current_hour_start_epoch = Utc::now().timestamp().div_euclid(3_600) * 3_600;
    let mut result = ParallelWorkActiveMinuteStats::empty_available();
    let scalar_end_epoch = complete_end_epoch.min(minute_keep_start_epoch);
    if complete_start_epoch < scalar_end_epoch {
        if complete_start_epoch.rem_euclid(3_600) != 0
            || scalar_end_epoch.rem_euclid(3_600) != 0
            || !parallel_work_hourly_coverage_is_complete(
                pool,
                complete_start_epoch,
                scalar_end_epoch,
                source_scope,
                "hourly_scalar_complete",
            )
            .await?
        {
            return Ok(ParallelWorkActiveMinuteStats::unavailable());
        }
        result = result.combine(
            query_parallel_work_hourly_scalar_stats(
                pool,
                complete_start_epoch,
                scalar_end_epoch,
                source_scope,
                upstream_account_id,
            )
            .await?,
        );
    }

    let minute_start_epoch = complete_start_epoch.max(minute_keep_start_epoch);
    let minute_end_epoch = complete_end_epoch.min(current_hour_start_epoch);
    if minute_start_epoch < minute_end_epoch {
        let coverage_start_epoch = minute_start_epoch.div_euclid(3_600) * 3_600;
        let coverage_end_epoch = (minute_end_epoch.saturating_add(3_599)).div_euclid(3_600) * 3_600;
        let minute_coverage_complete = parallel_work_hourly_coverage_is_complete(
            pool,
            coverage_start_epoch,
            coverage_end_epoch,
            source_scope,
            "minute_keys_complete",
        )
        .await?;
        let minute_stats = if minute_coverage_complete {
            ParallelWorkActiveMinuteStats::from_key_sets(
                query_parallel_work_minute_rollup_key_sets(
                    pool,
                    minute_start_epoch,
                    minute_end_epoch,
                    source_scope,
                    upstream_account_id,
                )
                .await?,
            )
        } else if raw_detail_start_epoch.is_some_and(|start| minute_start_epoch >= start) {
            let minute_start = Utc
                .timestamp_opt(minute_start_epoch, 0)
                .single()
                .ok_or_else(|| anyhow!("invalid parallel-work minute start"))?;
            let minute_end = Utc
                .timestamp_opt(minute_end_epoch, 0)
                .single()
                .ok_or_else(|| anyhow!("invalid parallel-work minute end"))?;
            ParallelWorkActiveMinuteStats::from_key_sets(
                query_parallel_work_exact_key_sets(
                    pool,
                    minute_start,
                    minute_end,
                    60,
                    chrono_tz::UTC,
                    source_scope,
                    upstream_account_id,
                    None,
                    None,
                )
                .await?,
            )
        } else {
            return Ok(ParallelWorkActiveMinuteStats::unavailable());
        };
        result = result.combine(minute_stats);
    }

    let raw_tail_start_epoch = complete_start_epoch.max(current_hour_start_epoch);
    if raw_tail_start_epoch < complete_end_epoch {
        let raw_tail_start = Utc
            .timestamp_opt(raw_tail_start_epoch, 0)
            .single()
            .ok_or_else(|| anyhow!("invalid parallel-work raw-tail start"))?;
        result = result.combine(ParallelWorkActiveMinuteStats::from_key_sets(
            query_parallel_work_exact_key_sets(
                pool,
                raw_tail_start,
                complete_end,
                60,
                chrono_tz::UTC,
                source_scope,
                upstream_account_id,
                None,
                None,
            )
            .await?,
        ));
    }
    Ok(result)
}

pub(crate) async fn query_parallel_work_exact_key_sets(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    start_after_id: Option<i64>,
    snapshot_id: Option<i64>,
) -> Result<BTreeMap<i64, HashSet<String>>> {
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

pub(crate) fn classify_invocation_failure_with_kind(
    status: Option<&str>,
    error_message: Option<&str>,
    explicit_failure_kind: Option<&str>,
) -> FailureClassification {
    let status_norm = status.unwrap_or_default().trim().to_ascii_lowercase();
    let err = error_message.unwrap_or_default().trim();
    let err_lower = err.to_ascii_lowercase();
    let explicit_failure_kind = explicit_failure_kind
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if (status_norm == "success"
        || status_norm == "completed"
        || status_norm == INVOCATION_STATUS_WARNING_SUCCESS)
        && err.is_empty()
        && explicit_failure_kind.is_none()
    {
        return FailureClassification {
            failure_kind: None,
            failure_class: FailureClass::None,
            is_actionable: false,
        };
    }
    if (status_norm == "running" || status_norm == "pending") && err.is_empty() {
        return FailureClassification {
            failure_kind: None,
            failure_class: FailureClass::None,
            is_actionable: false,
        };
    }
    if status_norm.is_empty() && err.is_empty() && explicit_failure_kind.is_none() {
        return FailureClassification {
            failure_kind: None,
            failure_class: FailureClass::None,
            is_actionable: false,
        };
    }

    let failure_kind = explicit_failure_kind
        .map(ToOwned::to_owned)
        .or_else(|| extract_failure_kind_prefix(err))
        .or_else(|| derive_failure_kind(&status_norm, err, &err_lower));

    let failure_kind_lower = failure_kind
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_http_429 =
        status_norm == "http_429" || failure_kind_lower == FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429;
    let is_http_4xx = (status_norm.starts_with("http_4")
        || status_norm == "http_401"
        || status_norm == "http_403")
        && !is_http_429;
    let is_http_5xx = status_norm.starts_with("http_5");

    let warning_success_like = status_norm == INVOCATION_STATUS_WARNING_SUCCESS
        && failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        && err.is_empty();

    let failure_class = if warning_success_like {
        FailureClass::None
    } else if failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
        || err_lower.contains("downstream closed while streaming upstream response")
    {
        FailureClass::ClientAbort
    } else if is_http_429 {
        // Upstream rate limiting is retryable and should be surfaced as service-impacting.
        FailureClass::ServiceFailure
    } else if failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED
        || err_lower.contains("invalid api key format")
        || err_lower.contains("api key format is invalid")
        || err_lower.contains("incorrect api key provided")
        || err_lower.contains("api key not found")
        || err_lower.contains("please provide an api key")
        || is_http_4xx
    {
        FailureClass::ClientFailure
    } else if failure_kind_lower == PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
        || failure_kind_lower == PROXY_FAILURE_PROXY_CONCURRENCY_LIMIT
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_STREAM_ERROR
        || failure_kind_lower == PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT
        || failure_kind_lower == PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
        || err_lower.contains("upstream response stream reported failure")
        || err_lower.contains("failed to contact upstream")
        || err_lower.contains("upstream stream error")
        || err_lower.contains("request body read timed out")
        || err_lower.contains("upstream handshake timed out")
        || is_http_5xx
    {
        FailureClass::ServiceFailure
    } else if (matches!(status_norm.as_str(), "success" | "completed" | "http_200")
        && err.is_empty()
        && failure_kind_lower.is_empty())
        || (status_norm == INVOCATION_STATUS_WARNING_SUCCESS
            && err.is_empty()
            && (failure_kind_lower.is_empty()
                || failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED))
    {
        FailureClass::None
    } else {
        // Conservative fallback: unknown non-success records are treated as service-impacting.
        FailureClass::ServiceFailure
    };

    FailureClassification {
        failure_kind: if failure_class == FailureClass::None
            && status_norm != INVOCATION_STATUS_WARNING_SUCCESS
        {
            None
        } else {
            failure_kind
        },
        failure_class,
        is_actionable: failure_class == FailureClass::ServiceFailure,
    }
}

pub(crate) fn classify_invocation_failure(
    status: Option<&str>,
    error_message: Option<&str>,
) -> FailureClassification {
    classify_invocation_failure_with_kind(status, error_message, None)
}

pub(crate) fn resolve_failure_classification(
    status: Option<&str>,
    error_message: Option<&str>,
    failure_kind: Option<&str>,
    failure_class: Option<&str>,
    is_actionable: Option<i64>,
) -> FailureClassification {
    let derived = classify_invocation_failure_with_kind(status, error_message, failure_kind);
    let stored_class = failure_class.and_then(FailureClass::from_db_str);
    let resolved_class = match stored_class {
        // Legacy rows can carry migration defaults (`none`/`0`) for non-success records.
        Some(FailureClass::None) if derived.failure_class != FailureClass::None => {
            derived.failure_class
        }
        Some(value) => value,
        None => derived.failure_class,
    };
    let resolved_kind = failure_kind
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned)
        .or(derived.failure_kind);
    let expected_actionable = resolved_class == FailureClass::ServiceFailure;
    let resolved_actionable = is_actionable
        .map(|value| value != 0)
        .filter(|value| *value == expected_actionable)
        .unwrap_or(expected_actionable);

    FailureClassification {
        failure_kind: resolved_kind,
        failure_class: resolved_class,
        is_actionable: resolved_actionable,
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ErrorQuery {
    pub(crate) range: String,
    pub(crate) top: Option<i64>,
    pub(crate) scope: Option<String>,
    pub(crate) time_zone: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorDistributionItem {
    pub(crate) reason: String,
    pub(crate) count: i64,
}

#[derive(serde::Serialize)]
pub(crate) struct ErrorDistributionResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) items: Vec<ErrorDistributionItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OtherErrorsQuery {
    pub(crate) range: String,
    pub(crate) page: Option<i64>,
    pub(crate) limit: Option<i64>,
    pub(crate) scope: Option<String>,
    pub(crate) time_zone: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct OtherErrorItem {
    pub(crate) id: i64,
    pub(crate) occurred_at: String,
    pub(crate) error_message: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct OtherErrorsResponse {
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) limit: i64,
    pub(crate) items: Vec<OtherErrorItem>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FailureSummaryQuery {
    pub(crate) range: String,
    pub(crate) time_zone: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FailureSummaryResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) total_failures: i64,
    pub(crate) service_failure_count: i64,
    pub(crate) client_failure_count: i64,
    pub(crate) client_abort_count: i64,
    pub(crate) actionable_failure_count: i64,
    pub(crate) actionable_failure_rate: f64,
}

pub(crate) async fn query_invocation_failure_hourly_rollup_range_tx(
    tx: &mut SqliteConnection,
    range_start_epoch: i64,
    range_end_epoch: i64,
    source_scope: InvocationSourceScope,
) -> Result<Vec<InvocationFailureHourlyRollupRecord>, ApiError> {
    let mut query = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            failure_class,
            is_actionable,
            error_category,
            SUM(failure_count) AS failure_count
        FROM invocation_failure_rollup_hourly
        WHERE bucket_start_epoch >=
        "#,
    );
    query.push_bind(range_start_epoch);
    query
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end_epoch);
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY failure_class, is_actionable, error_category");

    query
        .build_query_as::<InvocationFailureHourlyRollupRecord>()
        .fetch_all(&mut *tx)
        .await
        .map_err(Into::into)
}

pub(crate) async fn fetch_error_distribution(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ErrorQuery>,
) -> Result<Json<ErrorDistributionResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if start_dt < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let mut counts: HashMap<String, i64> = HashMap::new();
        let range_plan = build_hourly_rollup_exact_range_plan(
            start_dt,
            display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        let (hourly_rows, exact_records, archive_overlap_ids) =
            if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
                let mut tx = state.pool.begin().await?;
                let snapshot_id =
                    resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
                let rollup_live_cursor =
                    load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
                let hourly_rows = query_invocation_failure_hourly_rollup_range_tx(
                    tx.as_mut(),
                    range_start_epoch,
                    range_end_epoch,
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
                (hourly_rows, exact_records, archive_overlap_ids)
            } else {
                let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
                let exact_records = query_invocation_exact_records(
                    &state.pool,
                    &range_plan,
                    source_scope,
                    snapshot_id,
                )
                .await?;
                (Vec::new(), exact_records, HashSet::new())
            };
        for row in hourly_rows {
            let Some(class) = FailureClass::from_db_str(&row.failure_class) else {
                continue;
            };
            if !failure_scope_matches(scope, class) {
                continue;
            }
            *counts.entry(row.error_category).or_default() += row.failure_count;
        }
        for record in exact_records {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            if !failure_scope_matches(scope, classification.failure_class) {
                continue;
            }
            let raw = record.error_message.unwrap_or_default();
            let key = categorize_error(&raw);
            *counts.entry(key).or_default() += 1;
        }
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let archived_start = Utc
                .timestamp_opt(range_start_epoch, 0)
                .single()
                .ok_or_else(|| {
                    ApiError::from(anyhow!("invalid error distribution archive start epoch"))
                })?;
            let archived_end = Utc
                .timestamp_opt(range_end_epoch, 0)
                .single()
                .ok_or_else(|| {
                    ApiError::from(anyhow!("invalid error distribution archive end epoch"))
                })?;
            let archived_rows = crate::stats::load_unmaterialized_invocation_archive_failure_rows(
                &state.pool,
                archived_start,
                archived_end,
                source_scope,
                Some(&archive_overlap_ids),
            )
            .await?;
            for row in archived_rows {
                let classification = resolve_failure_classification(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                    row.failure_kind.as_deref(),
                    row.failure_class.as_deref(),
                    row.is_actionable,
                );
                if !failure_scope_matches(scope, classification.failure_class) {
                    continue;
                }
                let raw = row.error_message.unwrap_or_default();
                let key = categorize_error(&raw);
                *counts.entry(key).or_default() += 1;
            }
        }
        let mut items: Vec<ErrorDistributionItem> = counts
            .into_iter()
            .map(|(reason, count)| ErrorDistributionItem { reason, count })
            .collect();
        items.sort_by_key(|item| std::cmp::Reverse(item.count));
        if let Some(top) = params.top {
            let limited = top.clamp(1, 50) as usize;
            if items.len() > limited {
                items.truncate(limited);
            }
        }
        return Ok(Json(ErrorDistributionResponse {
            range_start: format_utc_iso(start_dt),
            range_end: format_utc_iso(display_end),
            items,
        }));
    }

    #[derive(sqlx::FromRow)]
    struct RawErr {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success')");
    let rows: Vec<RawErr> = query.build_query_as().fetch_all(&state.pool).await?;

    let mut counts: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for r in rows {
        let classification = resolve_failure_classification(
            r.status.as_deref(),
            r.error_message.as_deref(),
            r.failure_kind.as_deref(),
            r.failure_class.as_deref(),
            r.is_actionable,
        );
        if !failure_scope_matches(scope, classification.failure_class) {
            continue;
        }
        let raw = r.error_message.unwrap_or_default();
        let key = categorize_error(&raw);
        *counts.entry(key).or_insert(0) += 1;
    }

    let mut items: Vec<ErrorDistributionItem> = counts
        .into_iter()
        .map(|(reason, count)| ErrorDistributionItem { reason, count })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.count));
    if let Some(top) = params.top {
        let limited = top.clamp(1, 50) as usize;
        if items.len() > limited {
            items.truncate(limited);
        }
    }

    Ok(Json(ErrorDistributionResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        items,
    }))
}

// Classify error message by rules:
// - If contains HTTP code >= 501, group as "HTTP <code>"
// - If 4xx: try to extract concrete type (json error.type or regex phrases); otherwise "HTTP <code>"
// - Otherwise: normalize message and if still not matched, return "Other"
pub(crate) fn categorize_error(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "Other".to_string();
    }

    if let Some(code) = extract_http_code(s) {
        if code >= 501 {
            return format!("HTTP {}", code);
        }
        if (400..500).contains(&code) {
            if let Some(t) = extract_json_error_type(s) {
                return t.to_string();
            }
            if RE_USAGE_NOT_INCLUDED.is_match(s) {
                return "usage_not_included".to_string();
            }
            if RE_USAGE_LIMIT_REACHED.is_match(s) {
                return "usage_limit_reached".to_string();
            }
            if code == 429 {
                if RE_TOO_MANY_REQUESTS.is_match(s) {
                    return "too_many_requests".to_string();
                }
                return "http_429".to_string();
            }
            if code == 401 {
                return "unauthorized".to_string();
            }
            if code == 403 {
                return "forbidden".to_string();
            }
            if code == 404 {
                return "not_found".to_string();
            }
            return format!("HTTP {}", code);
        }
    }

    // Fallback to normalized text; if empty -> Other
    let norm = normalize_error_reason(s);
    if norm == "Unknown" || norm.is_empty() {
        "Other".to_string()
    } else {
        norm
    }
}

pub(crate) fn normalize_error_reason(input: &str) -> String {
    let s = input.trim();
    if s.is_empty() {
        return "Unknown".to_string();
    }
    // Extract stable info from JSON payloads if present
    if s.starts_with('{')
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(s)
        && let Some(err) = v.get("error")
        && let Some(ty) = err.get("type").and_then(|x| x.as_str())
    {
        return format!("json error: {ty}");
    }

    let mut out = s.to_lowercase();

    static RE_HTTP: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bhttp\s*(\d{3})\b").expect("valid regex"));
    let status = RE_HTTP
        .captures(&out)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());

    static RE_ISO_DT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b\d{4}-\d{2}-\d{2}[ t]\d{2}:\d{2}:\d{2}(?:\.\d+)?z?\b").expect("valid regex")
    });
    out = RE_ISO_DT.replace_all(&out, "").into_owned();

    static RE_UUID: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("valid regex")
    });
    out = RE_UUID.replace_all(&out, "").into_owned();

    static RE_LONG_ID: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b[a-z0-9_\-]{10,}\b").expect("valid regex"));
    out = RE_LONG_ID.replace_all(&out, "").into_owned();

    static RE_URL: Lazy<Regex> =
        Lazy::new(|| Regex::new(r#"https?://[^\s'\"]+"#).expect("valid regex"));
    out = RE_URL
        .replace_all(&out, |caps: &regex::Captures| {
            let url = &caps[0];
            if let Ok(u) = reqwest::Url::parse(url) {
                format!(
                    "{}://{}{}",
                    u.scheme(),
                    u.host_str().unwrap_or(""),
                    u.path()
                )
            } else {
                String::new()
            }
        })
        .into_owned();

    static RE_BIG_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b\d{4,}\b").expect("valid regex"));
    out = RE_BIG_NUM.replace_all(&out, "").into_owned();

    out = out.replace("request failed:", "request failed");
    out = out.replace("exception recovered:", "exception");

    static RE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").expect("valid regex"));
    out = RE_WS.replace_all(&out, " ").trim().to_string();

    if let Some(code) = status.as_ref().filter(|c| !out.contains(&c[..])) {
        out = format!("http {code}: {out}");
    }

    if out.is_empty() {
        "Unknown".to_string()
    } else {
        out.chars().take(160).collect()
    }
}

pub(crate) fn extract_http_code(s: &str) -> Option<u16> {
    static RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\bhttp\s*:?\s*(\d{3})\b").expect("valid regex"));
    RE.captures(s)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u16>().ok())
}

pub(crate) fn extract_json_error_type(s: &str) -> Option<String> {
    if !s.trim_start().starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let ty = v
        .get("error")
        .and_then(|e| e.get("type"))
        .and_then(|t| t.as_str())?;
    Some(ty.to_string())
}

pub(crate) static RE_USAGE_NOT_INCLUDED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*not[_\s-]*included").expect("valid regex"));
pub(crate) static RE_USAGE_LIMIT_REACHED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*limit[_\s-]*reached").expect("valid regex"));
pub(crate) static RE_TOO_MANY_REQUESTS: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)too\s+many\s+requests").expect("valid regex"));

pub(crate) async fn fetch_other_errors(
    State(state): State<Arc<AppState>>,
    Query(params): Query<OtherErrorsQuery>,
) -> Result<Json<OtherErrorsResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let scope = FailureScope::parse(params.scope.as_deref())?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    #[derive(sqlx::FromRow)]
    struct RowItem {
        id: i64,
        occurred_at: String,
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }
    let mut query = QueryBuilder::new(
        "SELECT id, occurred_at, status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" AND (status IS NULL OR status != 'success') ORDER BY occurred_at DESC");
    let rows: Vec<RowItem> = query.build_query_as().fetch_all(&state.pool).await?;

    let mut others: Vec<RowItem> = Vec::new();
    for r in rows.into_iter() {
        let classification = resolve_failure_classification(
            r.status.as_deref(),
            r.error_message.as_deref(),
            r.failure_kind.as_deref(),
            r.failure_class.as_deref(),
            r.is_actionable,
        );
        if !failure_scope_matches(scope, classification.failure_class) {
            continue;
        }
        let msg = r.error_message.clone().unwrap_or_default();
        let cat = categorize_error(&msg);
        if cat == "Other" {
            others.push(r);
        }
    }

    let total = others.len() as i64;
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let page = params.page.unwrap_or(1).max(1);
    let start = ((page - 1) * limit) as usize;
    let end = (start + limit as usize).min(others.len());
    let slice = if start < end {
        &others[start..end]
    } else {
        &[]
    };

    let items = slice
        .iter()
        .map(|r| OtherErrorItem {
            id: r.id,
            occurred_at: r.occurred_at.clone(),
            error_message: r.error_message.clone(),
        })
        .collect();

    Ok(Json(OtherErrorsResponse {
        total,
        page,
        limit,
        items,
    }))
}

pub(crate) async fn fetch_failure_summary(
    State(state): State<Arc<AppState>>,
    Query(params): Query<FailureSummaryQuery>,
) -> Result<Json<FailureSummaryResponse>, ApiError> {
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    let start_dt = range_window.start;
    let display_end = range_window.display_end;
    let source_scope = resolve_default_source_scope(&state.pool).await?;
    if start_dt < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let mut total_failures = 0_i64;
        let mut service_failure_count = 0_i64;
        let mut client_failure_count = 0_i64;
        let mut client_abort_count = 0_i64;
        let mut actionable_failure_count = 0_i64;
        let range_plan = build_hourly_rollup_exact_range_plan(
            start_dt,
            display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        let (hourly_rows, exact_records, archive_overlap_ids) =
            if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
                let mut tx = state.pool.begin().await?;
                let snapshot_id =
                    resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
                let rollup_live_cursor =
                    load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
                let hourly_rows = query_invocation_failure_hourly_rollup_range_tx(
                    tx.as_mut(),
                    range_start_epoch,
                    range_end_epoch,
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
                (hourly_rows, exact_records, archive_overlap_ids)
            } else {
                let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
                let exact_records = query_invocation_exact_records(
                    &state.pool,
                    &range_plan,
                    source_scope,
                    snapshot_id,
                )
                .await?;
                (Vec::new(), exact_records, HashSet::new())
            };
        for row in hourly_rows {
            let Some(class) = FailureClass::from_db_str(&row.failure_class) else {
                continue;
            };
            total_failures += row.failure_count;
            match class {
                FailureClass::ServiceFailure => service_failure_count += row.failure_count,
                FailureClass::ClientFailure => client_failure_count += row.failure_count,
                FailureClass::ClientAbort => client_abort_count += row.failure_count,
                FailureClass::None => {}
            }
            if row.is_actionable != 0 {
                actionable_failure_count += row.failure_count;
            }
        }
        for record in exact_records {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable,
            );
            if classification.failure_class == FailureClass::None {
                continue;
            }
            total_failures += 1;
            match classification.failure_class {
                FailureClass::ServiceFailure => service_failure_count += 1,
                FailureClass::ClientFailure => client_failure_count += 1,
                FailureClass::ClientAbort => client_abort_count += 1,
                FailureClass::None => {}
            }
            if classification.is_actionable {
                actionable_failure_count += 1;
            }
        }
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let archived_start = Utc
                .timestamp_opt(range_start_epoch, 0)
                .single()
                .ok_or_else(|| {
                    ApiError::from(anyhow!("invalid failure summary archive start epoch"))
                })?;
            let archived_end = Utc
                .timestamp_opt(range_end_epoch, 0)
                .single()
                .ok_or_else(|| {
                    ApiError::from(anyhow!("invalid failure summary archive end epoch"))
                })?;
            let archived_rows = crate::stats::load_unmaterialized_invocation_archive_failure_rows(
                &state.pool,
                archived_start,
                archived_end,
                source_scope,
                Some(&archive_overlap_ids),
            )
            .await?;
            for row in archived_rows {
                let classification = resolve_failure_classification(
                    row.status.as_deref(),
                    row.error_message.as_deref(),
                    row.failure_kind.as_deref(),
                    row.failure_class.as_deref(),
                    row.is_actionable,
                );
                if classification.failure_class == FailureClass::None {
                    continue;
                }
                total_failures += 1;
                match classification.failure_class {
                    FailureClass::ServiceFailure => service_failure_count += 1,
                    FailureClass::ClientFailure => client_failure_count += 1,
                    FailureClass::ClientAbort => client_abort_count += 1,
                    FailureClass::None => {}
                }
                if classification.is_actionable {
                    actionable_failure_count += 1;
                }
            }
        }
        let actionable_failure_rate = if total_failures > 0 {
            actionable_failure_count as f64 / total_failures as f64
        } else {
            0.0
        };
        return Ok(Json(FailureSummaryResponse {
            range_start: format_utc_iso(start_dt),
            range_end: format_utc_iso(display_end),
            total_failures,
            service_failure_count,
            client_failure_count,
            client_abort_count,
            actionable_failure_count,
            actionable_failure_rate,
        }));
    }

    #[derive(sqlx::FromRow)]
    struct Row {
        status: Option<String>,
        error_message: Option<String>,
        failure_kind: Option<String>,
        failure_class: Option<String>,
        is_actionable: Option<i64>,
    }

    let mut query = QueryBuilder::new(
        "SELECT status, error_message, failure_kind, failure_class, is_actionable FROM codex_invocations WHERE occurred_at >= ",
    );
    query.push_bind(db_occurred_at_lower_bound(start_dt));
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    let rows: Vec<Row> = query.build_query_as().fetch_all(&state.pool).await?;
    let mut total_failures = 0_i64;
    let mut service_failure_count = 0_i64;
    let mut client_failure_count = 0_i64;
    let mut client_abort_count = 0_i64;
    let mut actionable_failure_count = 0_i64;

    for row in rows {
        let classification = resolve_failure_classification(
            row.status.as_deref(),
            row.error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
            row.is_actionable,
        );
        if classification.failure_class == FailureClass::None {
            continue;
        }
        total_failures += 1;
        match classification.failure_class {
            FailureClass::ServiceFailure => service_failure_count += 1,
            FailureClass::ClientFailure => client_failure_count += 1,
            FailureClass::ClientAbort => client_abort_count += 1,
            FailureClass::None => {}
        }
        if classification.is_actionable {
            actionable_failure_count += 1;
        }
    }

    let actionable_failure_rate = if total_failures > 0 {
        actionable_failure_count as f64 / total_failures as f64
    } else {
        0.0
    };

    Ok(Json(FailureSummaryResponse {
        range_start: format_utc_iso(start_dt),
        range_end: format_utc_iso(display_end),
        total_failures,
        service_failure_count,
        client_failure_count,
        client_abort_count,
        actionable_failure_count,
        actionable_failure_rate,
    }))
}

pub(crate) async fn fetch_perf_stats(
    State(state): State<Arc<AppState>>,
    Query(params): Query<PerfQuery>,
) -> Result<Json<PerfStatsResponse>, ApiError> {
    #[derive(sqlx::FromRow)]
    struct PerfTimingRow {
        t_total_ms: Option<f64>,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
        t_upstream_ttfb_ms: Option<f64>,
        t_upstream_stream_ms: Option<f64>,
        t_resp_parse_ms: Option<f64>,
        t_persist_ms: Option<f64>,
    }

    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let range_window = resolve_range_window(&params.range, reporting_tz)?;
    if range_window.start < shanghai_retention_cutoff(state.config.invocation_max_days) {
        let range_plan = build_hourly_rollup_exact_range_plan(
            range_window.start,
            range_window.display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        let mut by_stage: BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)> =
            BTreeMap::new();
        let (exact_records, archive_overlap_ids) = if range_plan.full_hour_range.is_some() {
            let mut tx = state.pool.begin().await?;
            let snapshot_id =
                resolve_invocation_snapshot_id_tx(tx.as_mut(), InvocationSourceScope::ProxyOnly)
                    .await?;
            let rollup_live_cursor =
                load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
            let mut exact_records = query_invocation_exact_records_tx(
                tx.as_mut(),
                &range_plan,
                InvocationSourceScope::ProxyOnly,
                snapshot_id,
            )
            .await?;
            let tail_records = query_invocation_full_hour_tail_records_tx(
                tx.as_mut(),
                &range_plan,
                InvocationSourceScope::ProxyOnly,
                rollup_live_cursor,
                snapshot_id,
            )
            .await?;
            let archive_overlap_ids = tail_records
                .iter()
                .map(|record| record.id)
                .collect::<HashSet<_>>();
            exact_records.extend(tail_records);
            (exact_records, archive_overlap_ids)
        } else {
            let snapshot_id =
                resolve_invocation_snapshot_id(&state.pool, InvocationSourceScope::ProxyOnly)
                    .await?;
            (
                query_invocation_exact_records(
                    &state.pool,
                    &range_plan,
                    InvocationSourceScope::ProxyOnly,
                    snapshot_id,
                )
                .await?,
                HashSet::new(),
            )
        };
        if let Some((range_start_epoch, range_end_epoch)) = range_plan.full_hour_range {
            let rows = query_proxy_perf_stage_hourly_rollup_range(
                &state.pool,
                range_start_epoch,
                range_end_epoch,
            )
            .await?;
            for row in rows {
                let entry = by_stage
                    .entry(row.stage)
                    .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
                entry.0 += row.sample_count;
                entry.1 += row.sum_ms;
                entry.2 = entry.2.max(row.max_ms);
                merge_approx_histogram_into(
                    &mut entry.3,
                    &decode_approx_histogram(&row.histogram),
                )?;
            }
            let archived_start = Utc
                .timestamp_opt(range_start_epoch, 0)
                .single()
                .ok_or_else(|| ApiError::from(anyhow!("invalid perf archive start epoch")))?;
            let archived_end = Utc
                .timestamp_opt(range_end_epoch, 0)
                .single()
                .ok_or_else(|| ApiError::from(anyhow!("invalid perf archive end epoch")))?;
            let archived_perf =
                crate::stats::query_unmaterialized_proxy_perf_stage_rollups_from_archives(
                    &state.pool,
                    archived_start,
                    archived_end,
                    Some(&archive_overlap_ids),
                )
                .await?;
            for (stage, delta) in archived_perf {
                let entry = by_stage
                    .entry(stage)
                    .or_insert_with(|| (0, 0.0, 0.0, empty_approx_histogram()));
                entry.0 += delta.sample_count;
                entry.1 += delta.sum_ms;
                entry.2 = entry.2.max(delta.max_ms);
                merge_approx_histogram_into(&mut entry.3, &delta.histogram)?;
            }
        }
        for record in exact_records {
            record_perf_stage_sample(&mut by_stage, "total", record.t_total_ms);
            record_perf_stage_sample(&mut by_stage, "requestRead", record.t_req_read_ms);
            record_perf_stage_sample(&mut by_stage, "requestParse", record.t_req_parse_ms);
            record_perf_stage_sample(
                &mut by_stage,
                "upstreamConnect",
                record.t_upstream_connect_ms,
            );
            record_perf_stage_sample(
                &mut by_stage,
                "upstreamFirstByte",
                record.t_upstream_ttfb_ms,
            );
            record_perf_stage_sample(&mut by_stage, "upstreamStream", record.t_upstream_stream_ms);
            record_perf_stage_sample(&mut by_stage, "responseParse", record.t_resp_parse_ms);
            record_perf_stage_sample(&mut by_stage, "persistence", record.t_persist_ms);
        }
        let mut stages = Vec::new();
        for (stage, (count, sum_ms, max_ms, histogram)) in by_stage {
            if count <= 0 {
                continue;
            }
            stages.push(PerfStageStats {
                stage,
                count,
                avg_ms: sum_ms / count as f64,
                p50_ms: approx_histogram_percentile_ms(&histogram, 0.50).unwrap_or(max_ms),
                p90_ms: approx_histogram_percentile_ms(&histogram, 0.90).unwrap_or(max_ms),
                p99_ms: approx_histogram_percentile_ms(&histogram, 0.99).unwrap_or(max_ms),
                max_ms,
            });
        }
        return Ok(Json(PerfStatsResponse {
            range_start: format_utc_iso(range_window.start),
            range_end: format_utc_iso(range_window.display_end),
            source: SOURCE_PROXY.to_string(),
            stages,
        }));
    }
    let mut query = QueryBuilder::new(
        "SELECT \
            t_total_ms, t_req_read_ms, t_req_parse_ms, \
            t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, \
            t_resp_parse_ms, t_persist_ms \
         FROM codex_invocations \
         WHERE source = ",
    );
    query
        .push_bind(SOURCE_PROXY)
        .push(" AND occurred_at >= ")
        .push_bind(db_occurred_at_lower_bound(range_window.start))
        .push(" AND occurred_at <= ")
        .push_bind(db_occurred_at_lower_bound(range_window.display_end));
    let rows: Vec<PerfTimingRow> = query.build_query_as().fetch_all(&state.pool).await?;

    let stage_series: Vec<(&str, Vec<f64>)> = vec![
        (
            "total",
            rows.iter()
                .filter_map(|row| row.t_total_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "requestRead",
            rows.iter()
                .filter_map(|row| row.t_req_read_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "requestParse",
            rows.iter()
                .filter_map(|row| row.t_req_parse_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamConnect",
            rows.iter()
                .filter_map(|row| row.t_upstream_connect_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamFirstByte",
            rows.iter()
                .filter_map(|row| row.t_upstream_ttfb_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "upstreamStream",
            rows.iter()
                .filter_map(|row| row.t_upstream_stream_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "responseParse",
            rows.iter()
                .filter_map(|row| row.t_resp_parse_ms)
                .collect::<Vec<_>>(),
        ),
        (
            "persistence",
            rows.iter()
                .filter_map(|row| row.t_persist_ms)
                .collect::<Vec<_>>(),
        ),
    ];

    let mut stages = Vec::new();
    for (stage, mut values) in stage_series {
        if values.is_empty() {
            continue;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let count = values.len() as i64;
        let sum = values.iter().copied().sum::<f64>();
        let max_ms = values.last().copied().unwrap_or(0.0);
        stages.push(PerfStageStats {
            stage: stage.to_string(),
            count,
            avg_ms: sum / count as f64,
            p50_ms: percentile_sorted_f64(&values, 0.50),
            p90_ms: percentile_sorted_f64(&values, 0.90),
            p99_ms: percentile_sorted_f64(&values, 0.99),
            max_ms,
        });
    }

    Ok(Json(PerfStatsResponse {
        range_start: format_utc_iso(range_window.start),
        range_end: format_utc_iso(range_window.display_end),
        source: SOURCE_PROXY.to_string(),
        stages,
    }))
}

pub(crate) async fn latest_quota_snapshot(
    State(state): State<Arc<AppState>>,
) -> Result<Json<QuotaSnapshotResponse>, ApiError> {
    let snapshot = QuotaSnapshotResponse::fetch_latest(&state.pool)
        .await?
        .unwrap_or_else(QuotaSnapshotResponse::degraded_default);
    Ok(Json(snapshot))
}

pub(crate) async fn broadcast_quota_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    snapshot: QuotaSnapshotResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    if broadcaster.receiver_count() == 0 {
        return Ok(false);
    }

    let mut cache = cache.lock().await;
    if cache
        .quota
        .as_ref()
        .is_some_and(|current| current == &snapshot)
    {
        return Ok(false);
    }

    match broadcaster.send(BroadcastPayload::Quota {
        snapshot: Box::new(snapshot.clone()),
    }) {
        Ok(_) => {
            cache.quota = Some(snapshot);
            Ok(true)
        }
        Err(_err) if broadcaster.receiver_count() == 0 => Ok(false),
        Err(err) => Err(err),
    }
}

pub(crate) async fn sse_stream(
    state: State<Arc<AppState>>,
    query: Query<SubscriptionStreamQuery>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    topic_sse_stream(state, query).await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionResponse {
    pub(crate) backend: String,
    pub(crate) frontend: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_record(
        invoke_id: &str,
        account_id: Option<i64>,
        status: &str,
        phase: Option<&str>,
        attempts: i64,
    ) -> ApiInvocation {
        ApiInvocation {
            id: 1,
            invoke_id: invoke_id.to_string(),
            occurred_at: "2026-07-12 10:00:00".to_string(),
            source: SOURCE_PROXY.to_string(),
            proxy_display_name: None,
            model: None,
            request_model: None,
            response_model: None,
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            total_tokens: None,
            cost: None,
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            cache_write_tokens: None,
            status: Some(status.to_string()),
            live_phase: phase.map(str::to_string),
            error_message: None,
            downstream_status_code: None,
            failure_kind: None,
            blocked_binding: None,
            blocked_binding_json: None,
            stream_terminal_event: None,
            upstream_error_code: None,
            upstream_error_message: None,
            downstream_error_message: None,
            upstream_request_id: None,
            failure_class: None,
            is_actionable: None,
            endpoint: None,
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
            requester_ip: None,
            prompt_cache_key: None,
            sticky_key: None,
            route_mode: None,
            upstream_account_id: account_id,
            upstream_account_name: None,
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            pool_attempt_count: Some(attempts),
            pool_distinct_account_count: None,
            pool_attempt_terminal_reason: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            proxy_weight_delta: None,
            cost_estimated: None,
            price_version: None,
            cost_audit: None,
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: None,
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_pruned_at: None,
            detail_prune_reason: None,
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            created_at: "2026-07-12 10:00:00".to_string(),
        }
    }

    #[test]
    fn dashboard_activity_live_snapshot_groups_one_runtime_read_by_account() {
        let snapshot = build_dashboard_activity_live_snapshot(
            9,
            [
                live_record("c-1", Some(42), "running", Some("requesting"), 1),
                live_record("c-2", Some(42), "pending", Some("responding"), 2),
                live_record("u-1", None, "running", None, 1),
                live_record("done", Some(42), "success", None, 1),
            ],
        );

        assert_eq!(snapshot.revision, 9);
        assert_eq!(snapshot.in_progress_invocation_count, 3);
        assert_eq!(snapshot.retry_invocation_count, 1);
        assert_eq!(snapshot.in_progress_phase_counts.queued, 1);
        assert_eq!(snapshot.in_progress_phase_counts.requesting, 1);
        assert_eq!(snapshot.in_progress_phase_counts.responding, 1);
        assert_eq!(snapshot.accounts.len(), 2);
        let account = snapshot
            .accounts
            .iter()
            .find(|row| row.upstream_account_id == Some(42))
            .unwrap();
        assert_eq!(account.in_progress_invocation_count, 2);
        assert_eq!(account.retry_invocation_count, 1);
    }

    #[test]
    fn dashboard_activity_live_snapshot_infers_missing_runtime_phase() {
        let mut requesting = live_record("requesting", Some(42), "running", None, 1);
        requesting.t_upstream_connect_ms = Some(4.0);
        let mut responding = live_record("responding", Some(42), "running", None, 1);
        responding.t_upstream_ttfb_ms = Some(12.0);

        let snapshot = build_dashboard_activity_live_snapshot(10, [requesting, responding]);

        assert_eq!(snapshot.in_progress_phase_counts.queued, 0);
        assert_eq!(snapshot.in_progress_phase_counts.requesting, 1);
        assert_eq!(snapshot.in_progress_phase_counts.responding, 1);
        assert_eq!(snapshot.in_progress_wait_sample_count, 1);
        assert_eq!(snapshot.in_progress_wait_sum_ms, 12.0);
    }

    #[test]
    fn dashboard_activity_live_revision_reservation_is_monotonic() {
        let first = reserve_dashboard_activity_live_revision();
        let second = reserve_dashboard_activity_live_revision();

        assert_eq!(second, first + 1);
    }

    #[test]
    fn runtime_projection_mode_keeps_auto_default_and_legacy_kill_switch() {
        assert_eq!(
            RuntimeProjectionMode::parse(None).expect("default projection mode"),
            RuntimeProjectionMode::Auto
        );
        assert_eq!(
            RuntimeProjectionMode::parse(Some("legacy")).expect("legacy projection mode"),
            RuntimeProjectionMode::Legacy
        );
        assert!(RuntimeProjectionMode::parse(Some("invalid")).is_err());
    }

    #[test]
    fn runtime_projection_fixed_deadline_is_not_extended_by_later_mutations() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        let started_at = Instant::now();

        hub.mark_dashboard_dirty_at("runtime_upsert", started_at);
        let first_deadline = hub
            .pending_dashboard_deadline()
            .expect("first mutation should establish a deadline");
        hub.mark_dashboard_dirty_at("network_delta", started_at + Duration::from_millis(200));

        assert_eq!(
            first_deadline.duration_since(started_at),
            Duration::from_millis(250)
        );
        assert_eq!(hub.pending_dashboard_deadline(), Some(first_deadline));
    }

    #[test]
    fn terminal_rollback_marks_runtime_projection_dirty() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        let record = live_record("terminal-rollback", Some(42), "success", None, 1);
        let invoke_id = record.invoke_id.clone();
        let occurred_at = record.occurred_at.clone();

        hub.upsert_terminal(record);
        let generation_before_rollback = hub.dashboard_generation();

        assert!(hub.clear_terminal_tombstone(&invoke_id, &occurred_at));
        assert_eq!(hub.dashboard_generation(), generation_before_rollback + 1);
    }

    #[test]
    fn healthy_runtime_projection_renders_ten_thousand_mutations_without_sql() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(
            Utc::now(),
        )))
        .expect("bind dashboard network cache");
        let mut record = live_record("high-frequency", Some(42), "running", Some("queued"), 1);

        for mutation in 0..10_000 {
            record.live_phase = Some(if mutation % 2 == 0 {
                "requesting".to_string()
            } else {
                "responding".to_string()
            });
            hub.upsert(record.clone());
        }

        let mut render_samples_ms = Vec::new();
        let mut snapshot = None;
        for _ in 0..20 {
            let started_at = Instant::now();
            snapshot = Some(
                hub.dashboard_live_projection()
                    .snapshot()
                    .expect("healthy memory projection snapshot"),
            );
            render_samples_ms.push(started_at.elapsed().as_secs_f64() * 1_000.0);
        }
        render_samples_ms.sort_by(f64::total_cmp);
        let p95_index = ((render_samples_ms.len() - 1) as f64 * 0.95).ceil() as usize;
        let p95_ms = render_samples_ms[p95_index];
        let snapshot = snapshot.expect("projection snapshot");
        let health = hub.health_snapshot(0);

        assert_eq!(snapshot.in_progress_invocation_count, 1);
        assert_eq!(health.live_path_db_read_count, 0);
        assert!(
            p95_ms <= 400.0,
            "projection p95 exceeded 400ms: {p95_ms:.2}ms"
        );
    }

    #[test]
    fn runtime_projection_preserves_restored_rows_across_later_mutations() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(
            Utc::now(),
        )))
        .expect("bind dashboard network cache");
        let restored = live_record("restored-only", Some(41), "running", Some("requesting"), 1);
        let baseline_snapshot = build_dashboard_activity_live_snapshot(0, vec![restored.clone()]);
        let baseline = DashboardRuntimeProjectionBaseline {
            records: vec![DashboardRuntimeBaselineRecord {
                key: RuntimeInvocationKey::new(
                    restored.invoke_id.clone(),
                    restored.occurred_at.clone(),
                ),
                upstream_account_id: restored.upstream_account_id,
                upstream_account_name: restored.upstream_account_name.clone(),
                is_retry: false,
                live_phase: restored.live_phase.clone(),
                wait_ms: restored.t_upstream_ttfb_ms,
            }],
            source_scope: InvocationSourceScope::All,
        };
        hub.install_persistence_baseline_if_generation(
            baseline_snapshot,
            baseline,
            "startup_restore",
            hub.dashboard_generation(),
        )
        .expect("install startup baseline")
        .expect("baseline generation should match");

        hub.upsert(live_record(
            "runtime-only",
            Some(42),
            "running",
            Some("responding"),
            1,
        ));
        let capture = hub
            .capture_memory_snapshot()
            .expect("merged runtime projection snapshot");

        assert_eq!(capture.snapshot.in_progress_invocation_count, 2);
        assert_eq!(capture.snapshot.in_progress_phase_counts.requesting, 1);
        assert_eq!(capture.snapshot.in_progress_phase_counts.responding, 1);
        assert_eq!(hub.health_snapshot(1).live_path_db_read_count, 0);

        let mut restored_update = restored.clone();
        restored_update.live_phase = Some("responding".to_string());
        hub.upsert(restored_update.clone());
        let updated = hub
            .capture_memory_snapshot()
            .expect("runtime overlay should replace its restored row");
        assert_eq!(updated.snapshot.in_progress_invocation_count, 2);
        assert_eq!(updated.snapshot.in_progress_phase_counts.requesting, 0);
        assert_eq!(updated.snapshot.in_progress_phase_counts.responding, 2);

        restored_update.status = Some("completed".to_string());
        hub.upsert_terminal(restored_update);
        let terminal = hub
            .capture_memory_snapshot()
            .expect("terminal delta should remove its restored row");
        assert_eq!(terminal.snapshot.in_progress_invocation_count, 1);
        assert_eq!(terminal.snapshot.in_progress_phase_counts.responding, 1);
    }

    #[tokio::test]
    async fn dashboard_runtime_projection_update_p95_stays_within_four_hundred_ms() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let _lease = state
            .subscription_hub
            .register_test_topic_name("dashboard.activity.current")
            .await;
        let mut receiver = state.broadcaster.subscribe();
        let mut samples_ms = Vec::new();

        for mutation in 0..20 {
            let phase = if mutation % 2 == 0 {
                "requesting"
            } else {
                "responding"
            };
            state.proxy_runtime_invocations.upsert(live_record(
                "latency-contract",
                Some(42),
                "running",
                Some(phase),
                1,
            ));
            let started_at = Instant::now();
            schedule_dashboard_activity_live_snapshot(state.as_ref());
            let snapshot = tokio::time::timeout(Duration::from_millis(400), async {
                loop {
                    if let Ok(BroadcastPayload::DashboardActivityLive { snapshot }) =
                        receiver.recv().await
                    {
                        return snapshot;
                    }
                }
            })
            .await
            .expect("dashboard current update exceeded 400ms");
            assert_eq!(snapshot.in_progress_invocation_count, 1);
            samples_ms.push(started_at.elapsed().as_secs_f64() * 1_000.0);
        }

        samples_ms.sort_by(f64::total_cmp);
        let p95_index = ((samples_ms.len() - 1) as f64 * 0.95).ceil() as usize;
        let p95_ms = samples_ms[p95_index];
        let health = state.proxy_runtime_invocations.health_snapshot(1);
        assert_eq!(health.live_path_db_read_count, 0);
        assert!(
            p95_ms <= 400.0,
            "dashboard current update p95 exceeded 400ms: {p95_ms:.2}ms"
        );
    }

    #[tokio::test]
    async fn network_mutation_without_subscribers_is_dirty_for_first_snapshot() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        state.dashboard_network_speed_cache.record_request_bytes(
            "network-first-snapshot",
            "2026-08-04 12:00:00",
            Some(42),
            Some("api.openai.com"),
            128,
            Utc::now(),
        );

        schedule_dashboard_activity_live_snapshot(state.as_ref());
        assert!(
            state
                .proxy_runtime_invocations
                .pending_dashboard_deadline()
                .is_some()
        );
        let snapshot = capture_dashboard_activity_live_snapshot(state.as_ref())
            .await
            .expect("first memory snapshot after subscriber-free network mutation");
        let health = state.proxy_runtime_invocations.health_snapshot(0);

        assert_eq!(
            snapshot
                .network_live_bucket
                .expect("global live bucket")
                .upload_bytes,
            128
        );
        assert_eq!(health.live_path_db_read_count, 0);
        assert!(
            state
                .proxy_runtime_invocations
                .pending_dashboard_deadline()
                .is_none()
        );
    }

    #[test]
    fn unchanged_runtime_projection_does_not_advance_revision() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(
            Utc::now(),
        )))
        .expect("bind dashboard network cache");
        hub.upsert(live_record(
            "stable-revision",
            Some(42),
            "running",
            Some("requesting"),
            1,
        ));

        let first = hub
            .capture_memory_snapshot()
            .expect("first memory snapshot");
        let second = hub
            .capture_memory_snapshot()
            .expect("unchanged memory snapshot");

        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(second.snapshot.revision, first.snapshot.revision);
    }

    #[tokio::test]
    async fn degraded_runtime_projection_reuses_last_good_without_database_read() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        state.proxy_runtime_invocations.upsert(live_record(
            "last-good",
            Some(42),
            "running",
            Some("requesting"),
            1,
        ));
        let first = capture_dashboard_activity_live_snapshot(state.as_ref())
            .await
            .expect("healthy memory snapshot");
        state
            .proxy_runtime_invocations
            .mark_degraded("test_health_gate");

        let degraded = capture_dashboard_activity_live_snapshot(state.as_ref())
            .await
            .expect("degraded last-good snapshot");
        let health = state.proxy_runtime_invocations.health_snapshot(1);

        assert_eq!(degraded.revision, first.revision);
        assert_eq!(health.live_path_db_read_count, 0);
        assert_eq!(health.snapshot_origin, "last_good");
        assert_eq!(health.state, "degraded");
    }

    #[test]
    fn reconcile_failure_reports_degraded_health_without_freezing_memory_projection() {
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Auto);
        hub.bind_dashboard_network_speed_cache(Arc::new(DashboardNetworkSpeedCache::new(
            Utc::now(),
        )))
        .expect("bind dashboard network cache");
        hub.upsert(live_record(
            "reconcile-health",
            Some(42),
            "running",
            Some("requesting"),
            1,
        ));
        hub.record_reconcile_failure("reconcile_failed");

        let snapshot = hub
            .capture_memory_snapshot()
            .expect("reconcile failure must not gate healthy memory projection");
        let health = hub.health_snapshot(1);

        assert_eq!(snapshot.snapshot.in_progress_invocation_count, 1);
        assert!(hub.is_memory_ready());
        assert_eq!(health.state, "degraded");
        assert_eq!(health.degraded_reason.as_deref(), Some("reconcile_failed"));
        assert_eq!(health.live_path_db_read_count, 0);
    }

    #[tokio::test]
    async fn cold_runtime_projection_uses_exact_persistence_fallback_once() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;

        let snapshot = capture_dashboard_activity_live_snapshot(state.as_ref())
            .await
            .expect("cold persistence fallback");
        let health = state.proxy_runtime_invocations.health_snapshot(1);

        assert_eq!(snapshot.in_progress_invocation_count, 0);
        assert_eq!(health.live_path_db_read_count, 1);
        assert_eq!(health.snapshot_origin, "cold_fallback");
        assert_eq!(health.state, "healthy");
    }

    #[tokio::test]
    async fn legacy_runtime_projection_kill_switch_keeps_persistence_path() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let hub = RuntimeProjectionHub::new(RuntimeProjectionMode::Legacy);
        hub.bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
            .expect("bind dashboard network cache");

        let first = capture_dashboard_activity_live_snapshot_from_runtime(
            &state.pool,
            &hub,
            state.dashboard_network_speed_cache.as_ref(),
        )
        .await
        .expect("first legacy capture");
        let second = capture_dashboard_activity_live_snapshot_from_runtime(
            &state.pool,
            &hub,
            state.dashboard_network_speed_cache.as_ref(),
        )
        .await
        .expect("second legacy capture");
        let health = hub.health_snapshot(0);

        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(second.snapshot.revision, first.snapshot.revision);
        assert_eq!(health.mode, "legacy");
        assert_eq!(health.live_path_db_read_count, 2);
    }

    #[test]
    fn dashboard_activity_live_snapshot_serializes_network_realtime_rate() {
        let snapshot = DashboardActivityLiveSnapshot {
            revision: 11,
            generated_at: "2026-07-19T18:04:00.000Z".to_string(),
            in_progress_invocation_count: 0,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            network_live_bucket: None,
            network_realtime_rate: Some(DashboardNetworkRealtimeRateResponse {
                sample_start: "2026-07-19T18:03:59.000Z".to_string(),
                sample_end: "2026-07-19T18:04:00.000Z".to_string(),
                sample_seconds: 1,
                upload_bytes_per_second: 2048.0,
                download_bytes_per_second: 4096.0,
                upload_bytes: 2048,
                download_bytes: 4096,
            }),
            accounts: Vec::new(),
        };

        let payload = serde_json::to_value(&snapshot).expect("serialize dashboard activity live");

        assert_eq!(payload["networkRealtimeRate"]["sampleSeconds"], 1);
        assert_eq!(payload["networkRealtimeRate"]["uploadBytes"], 2048);
        assert_eq!(
            payload["networkRealtimeRate"]["downloadBytesPerSecond"],
            4096.0
        );
    }

    #[test]
    fn build_invocation_filters_normalizes_request_id() {
        let params = ListQuery {
            request_id: Some(" invoke-123 ".to_string()),
            ..Default::default()
        };

        let filters = build_invocation_filters(&params).expect("filters should build");

        assert_eq!(filters.request_id.as_deref(), Some("invoke-123"));
    }

    #[test]
    fn build_invocation_filters_ignores_legacy_proxy_param() {
        let params = ListQuery {
            proxy: Some(" tokyo-edge-01 ".to_string()),
            ..Default::default()
        };

        let filters = build_invocation_filters(&params).expect("filters should build");

        assert_eq!(params.proxy.as_deref(), Some(" tokyo-edge-01 "));
        assert_eq!(filters.endpoint, None);
        assert_eq!(filters.request_id, None);
    }

    #[test]
    fn response_body_falls_back_to_preview_when_complete() {
        let row = InvocationResponseBodyRow {
            id: 1,
            invoke_id: "invoke-preview".to_string(),
            payload: None,
            raw_response: "{\"error\":\"preview\"}".to_string(),
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: Some(19),
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_prune_reason: None,
            response_content_encoding: None,
            failure_class: Some("service_failure".to_string()),
            upstream_request_id: None,
            attempt_public_id: None,
        };

        let (body, from_full_body) =
            resolve_response_body_text_from_row(&row, None).expect("preview should be reusable");

        assert_eq!(body, "{\"error\":\"preview\"}");
        assert!(!from_full_body);
    }

    #[test]
    fn response_body_reports_detail_pruned_when_structured_only_preview_missing() {
        let row = InvocationResponseBodyRow {
            id: 2,
            invoke_id: "invoke-pruned".to_string(),
            payload: None,
            raw_response: String::new(),
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
            detail_prune_reason: Some("success_over_30d".to_string()),
            response_content_encoding: None,
            failure_class: Some("client_failure".to_string()),
            upstream_request_id: None,
            attempt_public_id: None,
        };

        let err = resolve_response_body_text_from_row(&row, None)
            .expect_err("structured-only rows should not expose a full body");

        assert_eq!(err, "detail_pruned");
    }

    #[test]
    fn attempt_response_body_reports_attempt_specific_missing_reason() {
        let row = InvocationResponseBodyRow {
            id: 3,
            invoke_id: "invoke-attempt-missing".to_string(),
            payload: None,
            raw_response: String::new(),
            request_raw_path: None,
            request_raw_size: None,
            request_raw_truncated: None,
            request_raw_truncated_reason: None,
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_prune_reason: None,
            response_content_encoding: None,
            failure_class: Some("service_failure".to_string()),
            upstream_request_id: None,
            attempt_public_id: Some("attempt-abc".to_string()),
        };

        assert_eq!(
            raw_response_fallback_reason(&row),
            "attempt_response_body_not_captured"
        );
    }
}

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let pricing = state.pricing_catalog.read().await.clone();
    let proxy = state.proxy_model_settings.read().await.clone();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref()).await?;
    Ok(Json(SettingsResponse {
        proxy: ProxyModelSettingsResponse::from_settings(proxy),
        forward_proxy,
        pricing: PricingSettingsResponse::from_catalog(&pricing),
    }))
}

pub(crate) async fn removed_proxy_model_settings_endpoint() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_FOUND,
        "endpoint removed; legacy reverse proxy settings are no longer supported",
    )
}

pub(crate) async fn put_proxy_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<ProxyModelSettingsUpdateRequest>,
) -> Result<Json<ProxyModelSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let ProxyModelSettingsUpdateRequest {
        hijack_enabled,
        merge_upstream_enabled,
        fast_mode_rewrite_mode: _legacy_fast_mode_rewrite_mode,
        upstream_429_max_retries,
        websocket_enabled,
        upstream_websocket_default_enabled,
        request_body_logging_enabled,
        response_body_logging_enabled,
        encrypted_session_owner_routing_enabled,
        enabled_models,
    } = payload;

    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
    let current = state.proxy_model_settings.read().await.clone();
    let next = ProxyModelSettings {
        hijack_enabled,
        merge_upstream_enabled,
        upstream_429_max_retries: upstream_429_max_retries
            .unwrap_or(current.upstream_429_max_retries),
        websocket_enabled: websocket_enabled.unwrap_or(current.websocket_enabled),
        upstream_websocket_default_enabled: upstream_websocket_default_enabled
            .unwrap_or(current.upstream_websocket_default_enabled),
        request_body_logging_enabled: request_body_logging_enabled
            .unwrap_or(current.request_body_logging_enabled),
        response_body_logging_enabled: response_body_logging_enabled
            .unwrap_or(current.response_body_logging_enabled),
        encrypted_session_owner_routing_enabled: encrypted_session_owner_routing_enabled
            .unwrap_or(current.encrypted_session_owner_routing_enabled),
        enabled_preset_models: enabled_models,
    }
    .normalized();
    save_proxy_model_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let mut guard = state.proxy_model_settings.write().await;
    *guard = next.clone();
    Ok(Json(ProxyModelSettingsResponse::from_settings(next)))
}

pub(crate) async fn put_pricing_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<PricingSettingsUpdateRequest>,
) -> Result<Json<PricingSettingsResponse>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin settings writes are forbidden".to_string(),
        ));
    }

    let next = payload.normalized()?;
    let _update_guard = state.pricing_settings_update_lock.lock().await;
    save_pricing_catalog(&state.pool, &next)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let mut guard = state.pricing_catalog.write().await;
    *guard = next.clone();
    Ok(Json(PricingSettingsResponse::from_catalog(&next)))
}

pub(crate) async fn get_versions(
    State(state): State<Arc<AppState>>,
) -> Result<Json<VersionResponse>, ApiError> {
    let (backend, frontend) = detect_versions(state.config.static_dir.as_deref());
    Ok(Json(VersionResponse { backend, frontend }))
}

#[derive(Debug, Default)]
pub(crate) struct BroadcastStateCache {
    pub(crate) quota: Option<QuotaSnapshotResponse>,
}

static DASHBOARD_ACTIVITY_LIVE_REVISION: AtomicU64 = AtomicU64::new(0);
pub(crate) const DASHBOARD_RUNTIME_PROJECTION_RECONCILE_INTERVAL: Duration =
    Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityLiveAccount {
    pub(crate) account_key: String,
    pub(crate) upstream_account_id: Option<i64>,
    #[serde(skip)]
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) in_progress_invocation_count: i64,
    pub(crate) in_progress_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) retry_invocation_count: i64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sum_ms: f64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sample_count: i64,
    pub(crate) upload_bytes_per_second: f64,
    pub(crate) download_bytes_per_second: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DashboardActivityLiveSnapshot {
    pub(crate) revision: u64,
    pub(crate) generated_at: String,
    pub(crate) in_progress_invocation_count: i64,
    pub(crate) in_progress_phase_counts: InvocationPhaseCountsResponse,
    pub(crate) retry_invocation_count: i64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sum_ms: f64,
    #[serde(skip)]
    pub(crate) in_progress_wait_sample_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_live_bucket: Option<DashboardNetworkTimeseriesPointResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) network_realtime_rate: Option<DashboardNetworkRealtimeRateResponse>,
    pub(crate) accounts: Vec<DashboardActivityLiveAccount>,
}

pub(crate) fn current_dashboard_activity_live_revision() -> u64 {
    DASHBOARD_ACTIVITY_LIVE_REVISION.load(Ordering::Acquire)
}

pub(crate) fn reserve_dashboard_activity_live_revision() -> u64 {
    DASHBOARD_ACTIVITY_LIVE_REVISION.fetch_add(1, Ordering::AcqRel) + 1
}

pub(crate) async fn capture_dashboard_activity_live_snapshot(
    state: &AppState,
) -> Result<DashboardActivityLiveSnapshot, ApiError> {
    let pending_window = state
        .proxy_runtime_invocations
        .pending_dashboard_publish_window();
    let capture = capture_dashboard_activity_live_snapshot_with_outcome(state).await?;
    if let Some(window) = pending_window {
        state
            .proxy_runtime_invocations
            .complete_dashboard_publish_window(window);
    }
    Ok(capture.snapshot)
}

async fn capture_dashboard_activity_live_snapshot_with_outcome(
    state: &AppState,
) -> Result<DashboardProjectionCapture, ApiError> {
    state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())?;
    capture_dashboard_activity_live_snapshot_from_runtime(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
    )
    .await
}

async fn capture_dashboard_activity_live_snapshot_from_runtime(
    pool: &Pool<Sqlite>,
    hub: &RuntimeProjectionHub,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
) -> Result<DashboardProjectionCapture, ApiError> {
    let started_at = Instant::now();
    let capture = match hub.mode() {
        RuntimeProjectionMode::Legacy => {
            capture_dashboard_activity_live_snapshot_from_persistence(
                pool,
                hub,
                dashboard_network_speed_cache,
                true,
                "legacy",
            )
            .await?
        }
        RuntimeProjectionMode::Auto if hub.is_memory_ready() => {
            match hub.capture_memory_snapshot() {
                Ok(capture) => capture,
                Err(err) => {
                    hub.mark_degraded("memory_snapshot_failed");
                    warn!(
                        ?err,
                        "dashboard runtime projection entered degraded last-good mode"
                    );
                    if let Some(capture) = hub.last_good_capture("last_good") {
                        capture
                    } else {
                        capture_dashboard_activity_live_snapshot_from_persistence(
                            pool,
                            hub,
                            dashboard_network_speed_cache,
                            true,
                            "cold_fallback",
                        )
                        .await?
                    }
                }
            }
        }
        RuntimeProjectionMode::Auto => {
            if let Some(capture) = hub.last_good_capture("last_good") {
                capture
            } else {
                capture_dashboard_activity_live_snapshot_from_persistence(
                    pool,
                    hub,
                    dashboard_network_speed_cache,
                    true,
                    "cold_fallback",
                )
                .await?
            }
        }
    };
    let health = hub.health_snapshot(0);
    tracing::debug!(
        projection = "dashboard_current",
        trigger = "capture",
        revision = capture.snapshot.revision,
        render_elapsed_ms = started_at.elapsed().as_millis() as u64,
        live_path_db_read_count = health.live_path_db_read_count,
        snapshot_origin = capture.snapshot_origin,
        last_good_age_ms = health.last_good_age_ms,
        changed = capture.changed,
        "captured dashboard runtime projection"
    );
    Ok(capture)
}

async fn capture_dashboard_activity_live_snapshot_from_persistence(
    pool: &Pool<Sqlite>,
    hub: &RuntimeProjectionHub,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    count_live_path_read: bool,
    snapshot_origin: &'static str,
) -> Result<DashboardProjectionCapture, ApiError> {
    let expected_generation = hub.dashboard_generation();
    if count_live_path_read {
        hub.record_live_path_db_read();
    }
    hub.record_build();
    let (snapshot, baseline) = query_dashboard_activity_live_snapshot_with_baseline_from_runtime(
        pool,
        hub,
        dashboard_network_speed_cache,
        0,
    )
    .await?;
    let expected_generation = if hub.mode() == RuntimeProjectionMode::Legacy {
        hub.dashboard_generation()
    } else {
        expected_generation
    };
    if let Some(capture) = hub.install_persistence_baseline_if_generation(
        snapshot,
        baseline,
        snapshot_origin,
        expected_generation,
    )? {
        return Ok(capture);
    }
    Ok(hub.capture_memory_snapshot()?)
}

pub(crate) async fn warm_dashboard_runtime_projection(state: &AppState) {
    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Legacy {
        return;
    }
    if let Err(err) = capture_dashboard_activity_live_snapshot_from_persistence(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
        false,
        "startup_restore",
    )
    .await
    {
        state
            .proxy_runtime_invocations
            .mark_degraded("startup_restore_failed");
        warn!(
            ?err,
            "failed to warm dashboard runtime projection from persistence"
        );
    }
}

pub(crate) async fn reconcile_dashboard_runtime_projection_once(
    state: &AppState,
) -> Result<DashboardProjectionCapture, ApiError> {
    capture_dashboard_activity_live_snapshot_from_persistence(
        &state.pool,
        state.proxy_runtime_invocations.as_ref(),
        state.dashboard_network_speed_cache.as_ref(),
        false,
        "reconcile",
    )
    .await
}

pub(crate) fn spawn_dashboard_runtime_projection_reconcile(state: Arc<AppState>) {
    if state.proxy_runtime_invocations.mode() == RuntimeProjectionMode::Legacy {
        return;
    }
    tokio::spawn(async move {
        let mut cadence = tokio::time::interval(DASHBOARD_RUNTIME_PROJECTION_RECONCILE_INTERVAL);
        cadence.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        cadence.tick().await;
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => return,
                _ = cadence.tick() => {}
            }
            let pressure_gate = crate::db_pressure::global_db_pressure_gate();
            let _pressure_permit = match pressure_gate
                .try_begin_background("dashboard_runtime_projection_reconcile")
            {
                Ok(permit) => permit,
                Err(reason) => {
                    let reason = match reason {
                        crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. } => {
                            "writer_pressure"
                        }
                        crate::db_pressure::DbPressureDenyReason::BackgroundBusy => {
                            "background_busy"
                        }
                    };
                    state
                        .proxy_runtime_invocations
                        .record_reconcile_deferred(reason);
                    tracing::debug!(
                        projection = "dashboard_current",
                        defer_reason = reason,
                        "deferred dashboard runtime projection reconcile"
                    );
                    continue;
                }
            };
            match reconcile_dashboard_runtime_projection_once(state.as_ref()).await {
                Ok(capture) => {
                    tracing::debug!(
                        projection = "dashboard_current",
                        revision = capture.snapshot.revision,
                        changed = capture.changed,
                        snapshot_origin = capture.snapshot_origin,
                        "reconciled dashboard runtime projection baseline"
                    );
                    if capture.changed
                        && state
                            .subscription_hub
                            .has_active_dashboard_activity_live_topic()
                            .await
                    {
                        let _ = state
                            .broadcaster
                            .send(BroadcastPayload::DashboardActivityLive {
                                snapshot: Box::new(capture.snapshot),
                            });
                    }
                }
                Err(err) => {
                    let pressure_error = match &err {
                        ApiError::BadRequest(err) | ApiError::Internal(err) => pressure_gate
                            .record_error("dashboard_runtime_projection_reconcile", err),
                    };
                    if pressure_error {
                        state
                            .proxy_runtime_invocations
                            .record_reconcile_deferred("writer_pressure");
                    } else {
                        state
                            .proxy_runtime_invocations
                            .record_reconcile_failure("reconcile_failed");
                    }
                    warn!(
                        ?err,
                        pressure_error, "failed to reconcile dashboard runtime projection baseline"
                    );
                }
            }
        }
    });
}

#[derive(Debug, Clone)]
struct DashboardProjectionInvocation {
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
    is_retry: bool,
    live_phase: Option<String>,
    wait_ms: Option<f64>,
}

fn build_dashboard_activity_live_snapshot_from_projection_records(
    revision: u64,
    records: impl IntoIterator<Item = DashboardProjectionInvocation>,
) -> DashboardActivityLiveSnapshot {
    let mut accounts = HashMap::<Option<i64>, DashboardActivityLiveAccount>::new();
    for record in records {
        let account_id = record.upstream_account_id;
        let account = accounts
            .entry(account_id)
            .or_insert_with(|| DashboardActivityLiveAccount {
                account_key: account_id
                    .map(|id| format!("upstream:{id}"))
                    .unwrap_or_else(|| "unassigned".to_string()),
                upstream_account_id: account_id,
                upstream_account_name: normalize_trimmed_optional_string_local(
                    record.upstream_account_name.clone(),
                ),
                in_progress_invocation_count: 0,
                in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
                retry_invocation_count: 0,
                in_progress_wait_sum_ms: 0.0,
                in_progress_wait_sample_count: 0,
                upload_bytes_per_second: 0.0,
                download_bytes_per_second: 0.0,
                network_live_bucket: None,
            });
        if account.upstream_account_name.is_none() {
            account.upstream_account_name =
                normalize_trimmed_optional_string_local(record.upstream_account_name.clone());
        }
        account.in_progress_invocation_count += 1;
        account
            .in_progress_phase_counts
            .increment_phase_name(record.live_phase.as_deref());
        if record.is_retry {
            account.retry_invocation_count += 1;
        }
        if let Some(wait_ms) = normalized_wait_ms(record.wait_ms) {
            account.in_progress_wait_sum_ms += wait_ms;
            account.in_progress_wait_sample_count += 1;
        }
    }
    let mut accounts = accounts.into_values().collect::<Vec<_>>();
    accounts.sort_by(|left, right| left.account_key.cmp(&right.account_key));
    let mut phase_counts = InvocationPhaseCountsResponse::default();
    let mut in_progress_invocation_count = 0;
    let mut retry_invocation_count = 0;
    let mut in_progress_wait_sum_ms = 0.0;
    let mut in_progress_wait_sample_count = 0;
    for account in &accounts {
        in_progress_invocation_count += account.in_progress_invocation_count;
        retry_invocation_count += account.retry_invocation_count;
        in_progress_wait_sum_ms += account.in_progress_wait_sum_ms;
        in_progress_wait_sample_count += account.in_progress_wait_sample_count;
        phase_counts.queued += account.in_progress_phase_counts.queued;
        phase_counts.requesting += account.in_progress_phase_counts.requesting;
        phase_counts.responding += account.in_progress_phase_counts.responding;
    }
    DashboardActivityLiveSnapshot {
        revision,
        generated_at: format_utc_iso(Utc::now()),
        in_progress_invocation_count,
        in_progress_phase_counts: phase_counts,
        retry_invocation_count,
        in_progress_wait_sum_ms,
        in_progress_wait_sample_count,
        network_live_bucket: None,
        network_realtime_rate: None,
        accounts,
    }
}

pub(crate) fn build_dashboard_activity_live_snapshot(
    revision: u64,
    records: impl IntoIterator<Item = ApiInvocation>,
) -> DashboardActivityLiveSnapshot {
    build_dashboard_activity_live_snapshot_from_projection_records(
        revision,
        records.into_iter().filter_map(|record| {
            matches!(
                normalized_runtime_text(record.status.as_deref()).as_str(),
                "running" | "pending"
            )
            .then(|| {
                let live_phase = record
                    .live_phase
                    .clone()
                    .or_else(|| runtime_invocation_live_phase(&record).map(str::to_string));
                DashboardProjectionInvocation {
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name,
                    is_retry: record.pool_attempt_count.unwrap_or_default() > 1,
                    live_phase,
                    wait_ms: record.t_upstream_ttfb_ms,
                }
            })
        }),
    )
}

pub(crate) fn build_dashboard_activity_live_snapshot_from_memory(
    revision: u64,
    baseline: Option<DashboardRuntimeProjectionBaseline>,
    runtime_records: impl IntoIterator<Item = ApiInvocation>,
    terminal_tombstones: HashSet<RuntimeInvocationKey>,
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
) -> DashboardActivityLiveSnapshot {
    let now = Utc::now();
    let source_scope = baseline
        .as_ref()
        .map_or(InvocationSourceScope::All, |baseline| baseline.source_scope);
    let mut projection_records = baseline
        .map(|baseline| {
            baseline
                .records
                .into_iter()
                .map(|record| {
                    (
                        record.key,
                        DashboardProjectionInvocation {
                            upstream_account_id: record.upstream_account_id,
                            upstream_account_name: record.upstream_account_name,
                            is_retry: record.is_retry,
                            live_phase: record.live_phase,
                            wait_ms: record.wait_ms,
                        },
                    )
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    for key in &terminal_tombstones {
        projection_records.remove(key);
    }
    for record in runtime_records {
        let key = RuntimeInvocationKey::new(record.invoke_id.clone(), record.occurred_at.clone());
        if terminal_tombstones.contains(&key) {
            projection_records.remove(&key);
            continue;
        }
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            continue;
        }
        if !matches!(
            normalized_runtime_text(record.status.as_deref()).as_str(),
            "running" | "pending"
        ) {
            projection_records.remove(&key);
            continue;
        }
        let baseline_record = projection_records.get(&key);
        let upstream_account_id = record
            .upstream_account_id
            .or_else(|| baseline_record.and_then(|record| record.upstream_account_id));
        let upstream_account_name = normalize_trimmed_optional_string_local(
            record.upstream_account_name.clone(),
        )
        .or_else(|| baseline_record.and_then(|record| record.upstream_account_name.clone()));
        let is_retry = record.pool_attempt_count.unwrap_or_default() > 1
            || baseline_record.is_some_and(|record| record.is_retry);
        let live_phase = record
            .live_phase
            .clone()
            .or_else(|| runtime_invocation_live_phase(&record).map(str::to_string));
        projection_records.insert(
            key,
            DashboardProjectionInvocation {
                upstream_account_id,
                upstream_account_name,
                is_retry,
                live_phase,
                wait_ms: record.t_upstream_ttfb_ms,
            },
        );
    }
    let mut snapshot = build_dashboard_activity_live_snapshot_from_projection_records(
        revision,
        projection_records.into_values(),
    );
    let account_rates = dashboard_network_speed_cache.snapshot_account_rates(now);
    let mut existing_account_keys = HashSet::new();
    for account in &mut snapshot.accounts {
        existing_account_keys.insert(account.account_key.clone());
        let rate = account_rates
            .get(&account.upstream_account_id)
            .copied()
            .unwrap_or_default();
        account.upload_bytes_per_second = rate.upload_bytes_per_second;
        account.download_bytes_per_second = rate.download_bytes_per_second;
        account.network_live_bucket = Some(dashboard_network_live_bucket_from_memory(
            dashboard_network_speed_cache,
            DashboardNetworkScopeKey::account_scope(account.upstream_account_id),
            now,
        ));
    }
    for (upstream_account_id, rate) in account_rates {
        let account_key = upstream_account_id
            .map(|id| format!("upstream:{id}"))
            .unwrap_or_else(|| "unassigned".to_string());
        if existing_account_keys.contains(&account_key) {
            continue;
        }
        snapshot.accounts.push(DashboardActivityLiveAccount {
            account_key,
            upstream_account_id,
            upstream_account_name: None,
            in_progress_invocation_count: 0,
            in_progress_phase_counts: InvocationPhaseCountsResponse::default(),
            retry_invocation_count: 0,
            in_progress_wait_sum_ms: 0.0,
            in_progress_wait_sample_count: 0,
            upload_bytes_per_second: rate.upload_bytes_per_second,
            download_bytes_per_second: rate.download_bytes_per_second,
            network_live_bucket: Some(dashboard_network_live_bucket_from_memory(
                dashboard_network_speed_cache,
                DashboardNetworkScopeKey::account_scope(upstream_account_id),
                now,
            )),
        });
    }
    snapshot
        .accounts
        .sort_by(|left, right| left.account_key.cmp(&right.account_key));
    snapshot.network_live_bucket = Some(dashboard_network_live_bucket_from_memory(
        dashboard_network_speed_cache,
        DashboardNetworkScopeKey::Global,
        now,
    ));
    snapshot.network_realtime_rate = Some(build_dashboard_network_realtime_rate_response(
        dashboard_network_speed_cache
            .snapshot_scope_realtime_bytes(DashboardNetworkScopeKey::Global, now),
    ));
    snapshot.generated_at = format_utc_iso(now);
    snapshot
}

fn dashboard_network_live_bucket_from_memory(
    dashboard_network_speed_cache: &DashboardNetworkSpeedCache,
    scope: DashboardNetworkScopeKey,
    now: DateTime<Utc>,
) -> DashboardNetworkTimeseriesPointResponse {
    let snapshot = dashboard_network_speed_cache.snapshot_open_bucket(scope, now);
    build_dashboard_network_timeseries_point_response(
        snapshot.bucket_start,
        snapshot.bucket_end,
        snapshot.totals,
        ExactUtcRange {
            start: snapshot.bucket_start,
            end: now.min(snapshot.bucket_end),
        },
        true,
    )
}

pub(crate) fn schedule_dashboard_activity_live_snapshot(state: &AppState) {
    if state.shutdown.is_cancelled() {
        return;
    }
    if let Err(err) = state
        .proxy_runtime_invocations
        .bind_dashboard_network_speed_cache(state.dashboard_network_speed_cache.clone())
    {
        state
            .proxy_runtime_invocations
            .mark_degraded("network_cache_bind_failed");
        warn!(
            ?err,
            "failed to bind dashboard network cache to runtime projection"
        );
        return;
    }
    state
        .proxy_runtime_invocations
        .mark_dashboard_dirty("dashboard_live_schedule");
    ensure_dashboard_activity_live_snapshot_producer(state);
}

pub(crate) fn ensure_dashboard_activity_live_snapshot_producer(state: &AppState) {
    if state.shutdown.is_cancelled()
        || state
            .proxy_runtime_invocations
            .pending_dashboard_publish_window()
            .is_none()
    {
        return;
    }
    if !state
        .subscription_hub
        .has_active_dashboard_activity_live_topic_sync()
    {
        return;
    }
    let _ = state
        .dashboard_activity_live_broadcast_seq
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    if state
        .dashboard_activity_live_broadcast_running
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    state.proxy_runtime_invocations.set_producer_running(true);

    let latest_seq = state.dashboard_activity_live_broadcast_seq.clone();
    let broadcast_running = state.dashboard_activity_live_broadcast_running.clone();
    let pool = state.pool.clone();
    let proxy_runtime_invocations = state.proxy_runtime_invocations.clone();
    let dashboard_network_speed_cache = state.dashboard_network_speed_cache.clone();
    let subscription_hub = state.subscription_hub.clone();
    let broadcaster = state.broadcaster.clone();
    let shutdown = state.shutdown.clone();
    tokio::spawn(async move {
        let mut delivered_seq = latest_seq.load(Ordering::Acquire).saturating_sub(1);
        loop {
            let Some(window) = proxy_runtime_invocations.pending_dashboard_publish_window() else {
                broadcast_running.store(false, Ordering::Release);
                proxy_runtime_invocations.set_producer_running(false);
                if proxy_runtime_invocations
                    .pending_dashboard_publish_window()
                    .is_some()
                    && broadcast_running
                        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                        .is_ok()
                {
                    proxy_runtime_invocations.set_producer_running(true);
                    continue;
                }
                return;
            };
            tokio::select! {
                _ = shutdown.cancelled() => {
                    broadcast_running.store(false, Ordering::Release);
                    proxy_runtime_invocations.set_producer_running(false);
                    return;
                }
                _ = tokio::time::sleep_until(tokio::time::Instant::from_std(window.deadline)) => {}
            }

            let sent_seq = latest_seq.load(Ordering::Acquire);
            let has_active_subscribers = subscription_hub
                .has_active_dashboard_activity_live_topic()
                .await;
            if has_active_subscribers {
                let started = Instant::now();
                match capture_dashboard_activity_live_snapshot_from_runtime(
                    &pool,
                    proxy_runtime_invocations.as_ref(),
                    dashboard_network_speed_cache.as_ref(),
                )
                .await
                {
                    Ok(capture) if capture.changed => {
                        let revision = capture.snapshot.revision;
                        if let Err(err) =
                            broadcaster.send(BroadcastPayload::DashboardActivityLive {
                                snapshot: Box::new(capture.snapshot),
                            })
                        {
                            warn!(
                                ?err,
                                revision, "failed to broadcast dashboard activity live snapshot"
                            );
                        } else {
                            tracing::debug!(
                                revision,
                                coalesced_mutation_count = sent_seq.saturating_sub(delivered_seq),
                                generated_to_sent_ms = started.elapsed().as_millis() as u64,
                                snapshot_origin = capture.snapshot_origin,
                                "broadcast dashboard activity live snapshot"
                            );
                        }
                    }
                    Ok(capture) => tracing::debug!(
                        revision = capture.snapshot.revision,
                        snapshot_origin = capture.snapshot_origin,
                        "suppressed unchanged dashboard activity live snapshot"
                    ),
                    Err(err) => warn!(?err, "failed to capture dashboard activity live snapshot"),
                }
            }
            proxy_runtime_invocations.complete_dashboard_publish_window(window);
            delivered_seq = sent_seq;

            if has_active_subscribers
                && dashboard_network_speed_cache
                    .should_keep_dashboard_activity_live_stream(Utc::now())
            {
                proxy_runtime_invocations.mark_dashboard_dirty("network_visibility");
            }
        }
    });
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum BroadcastPayload {
    Version {
        version: String,
    },
    Records {
        records: Vec<ApiInvocation>,
    },
    PromptCacheConversationChanged {
        prompt_cache_key: String,
    },
    PromptCacheConversationStickyRouteChanged {
        sticky_key: String,
        previous_upstream_account_id: i64,
        upstream_account_id: i64,
    },
    DashboardActivityLive {
        snapshot: Box<DashboardActivityLiveSnapshot>,
    },
    #[serde(rename = "pool_attempts")]
    PoolAttempts {
        invoke_id: String,
        attempts: Vec<ApiPoolUpstreamRequestAttempt>,
    },
    Quota {
        snapshot: Box<QuotaSnapshotResponse>,
    },
}

#[derive(Debug, Clone, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiInvocation {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    #[sqlx(default)]
    pub(crate) proxy_display_name: Option<String>,
    pub(crate) model: Option<String>,
    #[sqlx(default)]
    pub(crate) request_model: Option<String>,
    #[sqlx(default)]
    pub(crate) response_model: Option<String>,
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_input: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_cache_write: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_cache_read: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_output: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_reasoning: Option<f64>,
    #[sqlx(default)]
    pub(crate) cache_write_tokens: Option<i64>,
    pub(crate) status: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) live_phase: Option<String>,
    pub(crate) error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_status_code: Option<i64>,
    #[sqlx(default)]
    pub(crate) failure_kind: Option<String>,
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    #[sqlx(default)]
    #[serde(skip)]
    pub(crate) blocked_binding_json: Option<String>,
    #[sqlx(default)]
    pub(crate) stream_terminal_event: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_error_code: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_request_id: Option<String>,
    #[sqlx(default)]
    pub(crate) failure_class: Option<String>,
    #[sqlx(default)]
    pub(crate) is_actionable: Option<bool>,
    #[sqlx(default)]
    pub(crate) endpoint: Option<String>,
    #[sqlx(default)]
    pub(crate) compaction_request_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) compaction_response_kind: Option<String>,
    #[sqlx(default)]
    pub(crate) image_intent: Option<String>,
    #[sqlx(default)]
    pub(crate) requester_ip: Option<String>,
    #[sqlx(default)]
    pub(crate) prompt_cache_key: Option<String>,
    #[sqlx(default)]
    #[serde(skip_serializing)]
    pub(crate) sticky_key: Option<String>,
    #[sqlx(default)]
    pub(crate) route_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) response_content_encoding: Option<String>,
    #[sqlx(default)]
    pub(crate) request_compression_algorithm: Option<String>,
    #[sqlx(default)]
    pub(crate) transport: Option<String>,
    #[sqlx(default)]
    pub(crate) pool_attempt_count: Option<i64>,
    #[sqlx(default)]
    pub(crate) pool_distinct_account_count: Option<i64>,
    #[sqlx(default)]
    pub(crate) pool_attempt_terminal_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) requested_service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) billing_service_tier: Option<String>,
    #[sqlx(default)]
    pub(crate) proxy_weight_delta: Option<f64>,
    #[sqlx(default)]
    pub(crate) cost_estimated: Option<i64>,
    #[sqlx(default)]
    pub(crate) price_version: Option<String>,
    #[sqlx(default)]
    #[sqlx(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cost_audit: Option<InvocationCostAudit>,
    #[sqlx(default)]
    pub(crate) request_raw_path: Option<String>,
    #[sqlx(default)]
    pub(crate) request_raw_size: Option<i64>,
    #[sqlx(default)]
    pub(crate) request_raw_truncated: Option<i64>,
    #[sqlx(default)]
    pub(crate) request_raw_truncated_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) response_raw_path: Option<String>,
    #[sqlx(default)]
    pub(crate) response_raw_size: Option<i64>,
    #[sqlx(default)]
    pub(crate) response_raw_truncated: Option<i64>,
    #[sqlx(default)]
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) detail_level: String,
    #[sqlx(default)]
    #[serde(serialize_with = "serialize_opt_local_or_utc_to_utc_iso")]
    pub(crate) detail_pruned_at: Option<String>,
    #[sqlx(default)]
    pub(crate) detail_prune_reason: Option<String>,
    #[sqlx(default)]
    pub(crate) t_total_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_req_read_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_req_parse_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_upstream_connect_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_upstream_ttfb_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) first_token_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_upstream_stream_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_resp_parse_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_persist_ms: Option<f64>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationCostAuditBreakdown {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) input: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_write: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cache_read: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationCostAudit {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recorded: Option<InvocationCostAuditBreakdown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) local: Option<InvocationCostAuditBreakdown>,
    pub(crate) mismatch: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) absolute_diff_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) recorded_price_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) local_price_version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListResponse {
    pub(crate) snapshot_id: i64,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
    pub(crate) records: Vec<ApiInvocation>,
}
