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

fn resolve_parallel_work_rollup_reporting_tz(
    requested_reporting_tz: Tz,
    range_window: &RangeWindow,
) -> (Tz, bool) {
    if reporting_tz_has_whole_hour_offsets(requested_reporting_tz, range_window) {
        return (requested_reporting_tz, false);
    }
    (Shanghai, true)
}

fn build_parallel_work_window_response(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    bucket_seconds: i64,
    reporting_tz: Tz,
    counts_by_bucket: &BTreeMap<i64, i64>,
    effective_time_zone: Tz,
    time_zone_fallback: bool,
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
    let mut total = 0_f64;

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
        total += parallel_count as f64;
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
        min_count,
        max_count,
        avg_count: if complete_bucket_count > 0 {
            Some(total / complete_bucket_count as f64)
        } else {
            None
        },
        effective_time_zone: effective_time_zone.to_string(),
        time_zone_fallback,
        points,
    })
}

fn empty_parallel_work_window_response(
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
        min_count: None,
        max_count: None,
        avg_count: None,
        effective_time_zone: effective_time_zone.to_string(),
        time_zone_fallback,
        points: Vec::new(),
    }
}

async fn query_parallel_work_minute_counts(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
) -> Result<BTreeMap<i64, i64>> {
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
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY occurred_at ASC, prompt_cache_key ASC");

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
            align_reporting_bucket_epoch(occurred_at.timestamp(), 60, reporting_tz)?;
        bucket_keys
            .entry(bucket_start_epoch)
            .or_default()
            .insert(row.prompt_cache_key);
    }
    Ok(bucket_keys
        .into_iter()
        .map(|(bucket_start_epoch, prompt_cache_keys)| {
            (bucket_start_epoch, prompt_cache_keys.len() as i64)
        })
        .collect())
}

async fn query_parallel_work_hourly_counts(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    source_scope: InvocationSourceScope,
) -> Result<BTreeMap<i64, i64>> {
    let mut query = QueryBuilder::new(
        "SELECT bucket_start_epoch, COUNT(DISTINCT prompt_cache_key) AS parallel_count \
         FROM prompt_cache_rollup_hourly \
         WHERE bucket_start_epoch >= ",
    );
    query
        .push_bind(range_start.timestamp())
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end.timestamp())
        .push(" AND prompt_cache_key != ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" GROUP BY bucket_start_epoch ORDER BY bucket_start_epoch ASC");

    let rows = query
        .build_query_as::<ParallelWorkBucketCountRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.bucket_start_epoch, row.parallel_count.max(0)))
        .collect())
}

async fn resolve_parallel_work_day_all_window(
    pool: &Pool<Sqlite>,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
) -> Result<Option<RangeWindow>> {
    let now = Utc::now();
    let latest_complete_day_end =
        local_midnight_utc(now.with_timezone(&reporting_tz).date_naive(), reporting_tz);

    let earliest_bucket_epoch = if source_scope == InvocationSourceScope::ProxyOnly {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(bucket_start_epoch) FROM prompt_cache_rollup_hourly WHERE source = ?1 AND prompt_cache_key != ''",
        )
        .bind(SOURCE_PROXY)
        .fetch_one(pool)
        .await?
    } else {
        sqlx::query_scalar::<_, Option<i64>>(
            "SELECT MIN(bucket_start_epoch) FROM prompt_cache_rollup_hourly WHERE prompt_cache_key != ''",
        )
        .fetch_one(pool)
        .await?
    };

    let Some(earliest_bucket_epoch) = earliest_bucket_epoch else {
        return Ok(None);
    };
    let earliest_bucket = Utc
        .timestamp_opt(earliest_bucket_epoch, 0)
        .single()
        .ok_or_else(|| anyhow!("invalid earliest prompt-cache rollup bucket"))?;
    let earliest_local_date = earliest_bucket.with_timezone(&reporting_tz).date_naive();
    let first_day_start = local_midnight_utc(earliest_local_date, reporting_tz);
    let start = if first_day_start == earliest_bucket {
        first_day_start
    } else {
        let next_date = earliest_local_date
            .succ_opt()
            .unwrap_or(earliest_local_date + ChronoDuration::days(1));
        local_midnight_utc(next_date, reporting_tz)
    };

    if start >= latest_complete_day_end {
        return Ok(None);
    }

    Ok(Some(RangeWindow {
        start,
        end: latest_complete_day_end,
        display_end: latest_complete_day_end,
        duration: latest_complete_day_end.signed_duration_since(start),
    }))
}

async fn resolve_parallel_work_day_all_window_with_fallback(
    pool: &Pool<Sqlite>,
    requested_reporting_tz: Tz,
    source_scope: InvocationSourceScope,
) -> Result<(Option<RangeWindow>, Tz, bool)> {
    let requested_window =
        resolve_parallel_work_day_all_window(pool, requested_reporting_tz, source_scope).await?;
    if !should_fallback_parallel_work_day_all_window(
        requested_reporting_tz,
        requested_window.as_ref(),
        Utc::now(),
    ) {
        return Ok((requested_window, requested_reporting_tz, false));
    }
    Ok((
        resolve_parallel_work_day_all_window(pool, Shanghai, source_scope).await?,
        Shanghai,
        true,
    ))
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

async fn query_parallel_work_day_counts_from_hourly_rollups(
    pool: &Pool<Sqlite>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
) -> Result<BTreeMap<i64, i64>> {
    let mut query = QueryBuilder::new(
        "SELECT bucket_start_epoch, prompt_cache_key FROM prompt_cache_rollup_hourly \
         WHERE bucket_start_epoch >= ",
    );
    query
        .push_bind(range_start.timestamp())
        .push(" AND bucket_start_epoch < ")
        .push_bind(range_end.timestamp())
        .push(" AND prompt_cache_key != ''");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }
    query.push(" ORDER BY bucket_start_epoch ASC, prompt_cache_key ASC");

    let mut rows = query
        .build_query_as::<ParallelWorkDayRollupRow>()
        .fetch(pool);
    let mut counts = BTreeMap::new();
    let mut current_day_epoch = None;
    let mut current_keys = HashSet::new();

    while let Some(row) = rows.try_next().await? {
        let day_epoch = align_reporting_bucket_epoch(row.bucket_start_epoch, 86_400, reporting_tz)?;
        match current_day_epoch {
            Some(epoch) if epoch == day_epoch => {
                current_keys.insert(row.prompt_cache_key);
            }
            Some(epoch) => {
                counts.insert(epoch, current_keys.len() as i64);
                current_keys.clear();
                current_keys.insert(row.prompt_cache_key);
                current_day_epoch = Some(day_epoch);
            }
            None => {
                current_keys.insert(row.prompt_cache_key);
                current_day_epoch = Some(day_epoch);
            }
        }
    }

    if let Some(epoch) = current_day_epoch {
        counts.insert(epoch, current_keys.len() as i64);
    }

    Ok(counts)
}

fn local_naive_to_utc_not_after_reference(
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

fn classify_invocation_failure_with_kind(
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

    if (status_norm == "success" || status_norm == "completed")
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

    let failure_class = if failure_kind_lower == PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED
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
    } else if (status_norm == "success" || status_norm == "completed")
        && err.is_empty()
        && failure_kind_lower.is_empty()
    {
        FailureClass::None
    } else if status_norm == "http_200" && err.is_empty() && failure_kind_lower.is_empty() {
        FailureClass::None
    } else {
        // Conservative fallback: unknown non-success records are treated as service-impacting.
        FailureClass::ServiceFailure
    };

    FailureClassification {
        failure_kind: if failure_class == FailureClass::None {
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

async fn query_invocation_failure_hourly_rollup_range_tx(
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
        let (hourly_rows, exact_records) = if range_plan.full_hour_range.is_some() {
                let mut tx = state.pool.begin().await?;
                let snapshot_id =
                    resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
                let rollup_live_cursor =
                    load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
                let (range_start_epoch, range_end_epoch) = range_plan
                    .full_hour_range
                    .expect("full_hour_range is present when hourly rollups are enabled");
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
                exact_records.extend(
                    query_invocation_full_hour_tail_records_tx(
                        tx.as_mut(),
                        &range_plan,
                        source_scope,
                        rollup_live_cursor,
                        snapshot_id,
                    )
                    .await?,
                );
                (hourly_rows, exact_records)
            } else {
                let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
                let exact_records = query_invocation_exact_records(
                    &state.pool,
                    &range_plan,
                    source_scope,
                    snapshot_id,
                )
                .await?;
                (Vec::new(), exact_records)
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
        items.sort_by(|a, b| b.count.cmp(&a.count));
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
    items.sort_by(|a, b| b.count.cmp(&a.count));
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

static RE_USAGE_NOT_INCLUDED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*not[_\s-]*included").expect("valid regex"));
static RE_USAGE_LIMIT_REACHED: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)usage[_\s-]*limit[_\s-]*reached").expect("valid regex"));
static RE_TOO_MANY_REQUESTS: Lazy<Regex> =
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
        let (hourly_rows, exact_records) = if range_plan.full_hour_range.is_some() {
                let mut tx = state.pool.begin().await?;
                let snapshot_id =
                    resolve_invocation_snapshot_id_tx(tx.as_mut(), source_scope).await?;
                let rollup_live_cursor =
                    load_invocation_summary_rollup_live_cursor_tx(tx.as_mut()).await?;
                let (range_start_epoch, range_end_epoch) = range_plan
                    .full_hour_range
                    .expect("full_hour_range is present when hourly rollups are enabled");
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
                exact_records.extend(
                    query_invocation_full_hour_tail_records_tx(
                        tx.as_mut(),
                        &range_plan,
                        source_scope,
                        rollup_live_cursor,
                        snapshot_id,
                    )
                    .await?,
                );
                (hourly_rows, exact_records)
            } else {
                let snapshot_id = resolve_invocation_snapshot_id(&state.pool, source_scope).await?;
                let exact_records = query_invocation_exact_records(
                    &state.pool,
                    &range_plan,
                    source_scope,
                    snapshot_id,
                )
                .await?;
                (Vec::new(), exact_records)
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
                .ok_or_else(|| ApiError::from(anyhow!("invalid failure summary archive start epoch")))?;
            let archived_end = Utc
                .timestamp_opt(range_end_epoch, 0)
                .single()
                .ok_or_else(|| ApiError::from(anyhow!("invalid failure summary archive end epoch")))?;
            let archived_rows = crate::stats::load_unmaterialized_invocation_archive_failure_rows(
                &state.pool,
                archived_start,
                archived_end,
                source_scope,
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
        let snapshot_id =
            resolve_invocation_snapshot_id(&state.pool, InvocationSourceScope::ProxyOnly).await?;
        let range_plan = build_hourly_rollup_exact_range_plan(
            range_window.start,
            range_window.display_end,
            shanghai_retention_cutoff(state.config.invocation_max_days),
        )?;
        let mut by_stage: BTreeMap<String, (i64, f64, f64, ApproxHistogramCounts)> =
            BTreeMap::new();
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
        }
        let exact_records = query_invocation_exact_records(
            &state.pool,
            &range_plan,
            InvocationSourceScope::ProxyOnly,
            snapshot_id,
        )
        .await?;
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

pub(crate) async fn broadcast_summary_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    window: &str,
    summary: StatsResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    let mut cache = cache.lock().await;
    if cache
        .summaries
        .get(window)
        .is_some_and(|current| current == &summary)
    {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Summary {
        window: window.to_string(),
        summary: summary.clone(),
    })?;
    cache.summaries.insert(window.to_string(), summary);
    Ok(true)
}

pub(crate) async fn broadcast_quota_if_changed(
    broadcaster: &broadcast::Sender<BroadcastPayload>,
    cache: &Mutex<BroadcastStateCache>,
    snapshot: QuotaSnapshotResponse,
) -> Result<bool, broadcast::error::SendError<BroadcastPayload>> {
    let mut cache = cache.lock().await;
    if cache
        .quota
        .as_ref()
        .is_some_and(|current| current == &snapshot)
    {
        return Ok(false);
    }

    broadcaster.send(BroadcastPayload::Quota {
        snapshot: Box::new(snapshot.clone()),
    })?;
    cache.quota = Some(snapshot);
    Ok(true)
}

pub(crate) async fn sse_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.broadcaster.subscribe();
    let broadcast = BroadcastStream::new(rx).filter_map(|res| async {
        match res {
            Ok(payload) => match Event::default().json_data(&payload) {
                Ok(event) => Some(Ok(event)),
                Err(err) => {
                    warn!(?err, "failed to serialize sse payload");
                    None
                }
            },
            Err(err) => {
                warn!(?err, "sse broadcast stream lagging");
                None
            }
        }
    });
    // Seed a version event on connect so clients know the current server version immediately
    let initial = {
        let (backend, _frontend) = detect_versions(state.config.static_dir.as_deref());
        let payload = BroadcastPayload::Version { version: backend };
        let ev = Event::default().json_data(&payload);
        match ev {
            Ok(event) => stream::iter(vec![Ok(event)]),
            Err(_) => stream::iter(Vec::<Result<Event, Infallible>>::new()),
        }
    };

    let merged = initial.chain(broadcast);
    Sse::new(merged).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
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
            raw_response: "{\"error\":\"preview\"}".to_string(),
            response_raw_path: None,
            response_raw_size: Some(19),
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: "full".to_string(),
            detail_prune_reason: None,
            response_content_encoding: None,
            failure_class: Some("service_failure".to_string()),
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
            raw_response: String::new(),
            response_raw_path: None,
            response_raw_size: None,
            response_raw_truncated: Some(0),
            response_raw_truncated_reason: None,
            detail_level: DETAIL_LEVEL_STRUCTURED_ONLY.to_string(),
            detail_prune_reason: Some("success_over_30d".to_string()),
            response_content_encoding: None,
            failure_class: Some("client_failure".to_string()),
        };

        let err = resolve_response_body_text_from_row(&row, None)
            .expect_err("structured-only rows should not expose a full body");

        assert_eq!(err, "detail_pruned");
    }
}

pub(crate) async fn get_settings(
    State(state): State<Arc<AppState>>,
) -> Result<Json<SettingsResponse>, ApiError> {
    let pricing = state.pricing_catalog.read().await.clone();
    let forward_proxy = build_forward_proxy_settings_response(state.as_ref()).await?;
    Ok(Json(SettingsResponse {
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

    let _update_guard = state.proxy_model_settings_update_lock.lock().await;
    let current = state.proxy_model_settings.read().await.clone();
    let next = ProxyModelSettings {
        hijack_enabled: payload.hijack_enabled,
        merge_upstream_enabled: payload.merge_upstream_enabled,
        upstream_429_max_retries: payload
            .upstream_429_max_retries
            .unwrap_or(current.upstream_429_max_retries),
        enabled_preset_models: payload.enabled_models,
    }
    .normalized();
    save_proxy_model_settings(&state.pool, next.clone())
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let mut guard = state.proxy_model_settings.write().await;
    *guard = next.clone();
    Ok(Json(next.into()))
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
    pub(crate) summaries: HashMap<String, StatsResponse>,
    pub(crate) quota: Option<QuotaSnapshotResponse>,
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
    #[serde(rename = "pool_attempts")]
    PoolAttempts {
        invoke_id: String,
        attempts: Vec<ApiPoolUpstreamRequestAttempt>,
    },
    Summary {
        window: String,
        summary: StatsResponse,
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
    pub(crate) input_tokens: Option<i64>,
    pub(crate) output_tokens: Option<i64>,
    pub(crate) cache_input_tokens: Option<i64>,
    pub(crate) reasoning_tokens: Option<i64>,
    #[sqlx(default)]
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
    pub(crate) status: Option<String>,
    pub(crate) error_message: Option<String>,
    #[sqlx(default)]
    pub(crate) downstream_status_code: Option<i64>,
    #[sqlx(default)]
    pub(crate) failure_kind: Option<String>,
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
    pub(crate) requester_ip: Option<String>,
    #[sqlx(default)]
    pub(crate) prompt_cache_key: Option<String>,
    #[sqlx(default)]
    pub(crate) route_mode: Option<String>,
    #[sqlx(default)]
    pub(crate) upstream_account_id: Option<i64>,
    #[sqlx(default)]
    pub(crate) upstream_account_name: Option<String>,
    #[sqlx(default)]
    pub(crate) response_content_encoding: Option<String>,
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
    pub(crate) t_upstream_stream_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_resp_parse_ms: Option<f64>,
    #[sqlx(default)]
    pub(crate) t_persist_ms: Option<f64>,
    #[serde(serialize_with = "serialize_local_naive_to_utc_iso")]
    pub(crate) created_at: String,
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
