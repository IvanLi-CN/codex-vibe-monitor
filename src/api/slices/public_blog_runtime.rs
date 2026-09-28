use super::*;
use chrono::Timelike;
use std::future::Future;

const PUBLIC_BLOG_RUNTIME_PATH: &str = "/api/public/blog-runtime/v1/codex-vibe-monitor";
const PUBLIC_BLOG_RUNTIME_KIND: &str = "codex-vibe-monitor";
const PUBLIC_BLOG_SNAPSHOT_TTL: Duration = Duration::from_secs(30);
const PUBLIC_BLOG_REFRESH_TIMEOUT: Duration = Duration::from_secs(3);
const PUBLIC_BLOG_RATE_CAPACITY: f64 = 120.0;
const PUBLIC_BLOG_RATE_PER_SECOND: f64 = 10.0;
const PUBLIC_BLOG_CACHE_CONTROL: &str =
    "public, max-age=15, s-maxage=30, stale-while-revalidate=30, stale-if-error=300";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicBlogRuntimeResponse {
    kind: &'static str,
    tokens_per_minute: PublicBlogStat,
    parallel_calls: PublicBlogStat,
    today_tokens: PublicBlogStat,
    token_activity90d: Vec<PublicBlogDailyPoint>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PublicBlogStat {
    value: f64,
    trend: PublicBlogTrend,
}

#[derive(Debug, Clone, Serialize)]
struct PublicBlogTrend {
    range: &'static str,
    points: Vec<PublicBlogTrendPoint>,
}

#[derive(Debug, Clone, Serialize)]
struct PublicBlogTrendPoint {
    timestamp: String,
    value: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct PublicBlogDailyPoint {
    date: String,
    value: i64,
}

#[derive(Debug)]
struct CachedPublicBlogSnapshot {
    body: Bytes,
    etag: String,
    fresh_until: Instant,
}

#[derive(Debug)]
struct PublicBlogRateLimiter {
    tokens: f64,
    last_refill: Instant,
}

impl Default for PublicBlogRateLimiter {
    fn default() -> Self {
        Self {
            tokens: PUBLIC_BLOG_RATE_CAPACITY,
            last_refill: Instant::now(),
        }
    }
}

impl PublicBlogRateLimiter {
    fn take(&mut self, now: Instant) -> Option<Duration> {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.tokens =
            (self.tokens + elapsed * PUBLIC_BLOG_RATE_PER_SECOND).min(PUBLIC_BLOG_RATE_CAPACITY);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return None;
        }
        Some(Duration::from_secs_f64(
            ((1.0 - self.tokens) / PUBLIC_BLOG_RATE_PER_SECOND).max(0.001),
        ))
    }
}

#[derive(Debug)]
struct PublicBlogRuntimeCache {
    snapshot: RwLock<Option<Arc<CachedPublicBlogSnapshot>>>,
    refresh_lock: Mutex<()>,
    limiter: std::sync::Mutex<PublicBlogRateLimiter>,
}

impl Default for PublicBlogRuntimeCache {
    fn default() -> Self {
        Self {
            snapshot: RwLock::new(None),
            refresh_lock: Mutex::new(()),
            limiter: std::sync::Mutex::new(PublicBlogRateLimiter::default()),
        }
    }
}

impl PublicBlogRuntimeCache {
    fn take_request(&self) -> Option<Duration> {
        self.limiter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(Instant::now())
    }

    async fn fresh_snapshot(&self) -> Option<Arc<CachedPublicBlogSnapshot>> {
        self.snapshot
            .read()
            .await
            .as_ref()
            .filter(|snapshot| snapshot.fresh_until > Instant::now())
            .cloned()
    }

    async fn last_snapshot(&self) -> Option<Arc<CachedPublicBlogSnapshot>> {
        self.snapshot.read().await.clone()
    }

    #[cfg(test)]
    async fn expire_snapshot(&self) {
        let mut snapshot = self.snapshot.write().await;
        if let Some(current) = snapshot.as_ref() {
            *snapshot = Some(Arc::new(CachedPublicBlogSnapshot {
                body: current.body.clone(),
                etag: current.etag.clone(),
                fresh_until: Instant::now() - Duration::from_secs(1),
            }));
        }
    }

    async fn snapshot_or_refresh<F, Fut>(&self, refresh: F) -> Option<Arc<CachedPublicBlogSnapshot>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Bytes>>,
    {
        if let Some(snapshot) = self.fresh_snapshot().await {
            return Some(snapshot);
        }

        let deadline = Instant::now() + PUBLIC_BLOG_REFRESH_TIMEOUT;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _refresh_guard = match timeout(remaining, self.refresh_lock.lock()).await {
            Ok(guard) => guard,
            Err(_) => return self.last_snapshot().await,
        };
        if let Some(snapshot) = self.fresh_snapshot().await {
            return Some(snapshot);
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        let refreshed = timeout(remaining, refresh()).await;
        match refreshed {
            Ok(Ok(body)) => {
                let etag = format!("\"{:x}\"", Sha256::digest(&body));
                let snapshot = Arc::new(CachedPublicBlogSnapshot {
                    body,
                    etag,
                    fresh_until: Instant::now() + PUBLIC_BLOG_SNAPSHOT_TTL,
                });
                *self.snapshot.write().await = Some(snapshot.clone());
                Some(snapshot)
            }
            Ok(Err(_)) | Err(_) => self.last_snapshot().await,
        }
    }
}

pub(crate) fn build_public_blog_runtime_router(state: Arc<AppState>) -> Router {
    let cors =
        public_blog_runtime_cors_layer(&state.config.public_blog_runtime_cors_allowed_origins);
    Router::new()
        .route(PUBLIC_BLOG_RUNTIME_PATH, any(fetch_public_blog_runtime))
        .layer(cors)
        .layer(Extension(Arc::new(PublicBlogRuntimeCache::default())))
        .with_state(state)
}

fn public_blog_runtime_cors_layer(origins: &[String]) -> CorsLayer {
    let origins = origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect::<Vec<_>>();
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET])
        .allow_headers([HeaderName::from_static("if-none-match")])
        .expose_headers([
            HeaderName::from_static("etag"),
            HeaderName::from_static("cache-control"),
        ])
}

async fn fetch_public_blog_runtime(
    State(state): State<Arc<AppState>>,
    Extension(cache): Extension<Arc<PublicBlogRuntimeCache>>,
    request: Request<Body>,
) -> Response {
    if let Some(response) = reject_non_get(request.method()) {
        return response;
    }
    let headers = request.headers().clone();
    if let Some(retry_after) = cache.take_request() {
        let retry_after_seconds = retry_after.as_secs().max(1).to_string();
        let mut response =
            public_blog_error_response(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
        response.headers_mut().insert(
            HeaderName::from_static("retry-after"),
            HeaderValue::from_str(&retry_after_seconds)
                .expect("integer retry-after is a valid header"),
        );
        return response;
    }

    let snapshot = cache
        .snapshot_or_refresh(|| build_public_blog_runtime_snapshot(state))
        .await;
    let Some(snapshot) = snapshot else {
        return public_blog_error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "temporarily_unavailable",
        );
    };

    let not_modified = headers
        .get(HeaderName::from_static("if-none-match"))
        .is_some_and(|value| if_none_match_matches(value, &snapshot.etag));
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Response::new(Body::from(snapshot.body.clone()))
    };
    if !not_modified {
        response.headers_mut().insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    response.headers_mut().insert(
        HeaderName::from_static("etag"),
        HeaderValue::from_str(&snapshot.etag).expect("SHA-256 ETag is a valid header"),
    );
    response.headers_mut().insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static(PUBLIC_BLOG_CACHE_CONTROL),
    );
    response
}

fn public_blog_error_response(status: StatusCode, code: &'static str) -> Response {
    let mut response = (status, Json(json!({ "error": code }))).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static("no-store"),
    );
    response
}

fn reject_non_get(method: &Method) -> Option<Response> {
    if method == Method::GET {
        return None;
    }
    let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();
    response.headers_mut().insert(
        HeaderName::from_static("allow"),
        HeaderValue::from_static("GET"),
    );
    Some(response)
}

fn if_none_match_matches(header_value: &HeaderValue, etag: &str) -> bool {
    header_value.to_str().is_ok_and(|raw| {
        raw.split(',').any(|candidate| {
            let candidate = candidate.trim();
            candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
        })
    })
}

async fn build_public_blog_runtime_snapshot(state: Arc<AppState>) -> Result<Bytes> {
    let dashboard = fetch_dashboard_activity(
        State(state.clone()),
        Query(DashboardActivityQuery {
            range: "today".to_string(),
            recent_limit: Some(1),
            time_zone: Some("Asia/Shanghai".to_string()),
            include_accounts: false,
            include_recent: Some(false),
        }),
    )
    .await
    .map_err(|_| anyhow!("dashboard aggregates are unavailable"))?
    .0;
    let tokens_per_minute = finite_nonnegative(
        dashboard
            .summary
            .tokens_per_minute
            .ok_or_else(|| anyhow!("live token rate is unavailable"))?,
    )?;
    let parallel_calls = finite_nonnegative(
        dashboard
            .summary
            .stats
            .in_progress_conversation_count
            .ok_or_else(|| anyhow!("current parallel count is unavailable"))? as f64,
    )?;
    let today_tokens = finite_nonnegative(dashboard.summary.stats.total_tokens as f64)?;

    let timeseries = fetch_timeseries(
        State(state.clone()),
        Query(TimeseriesQuery {
            range: "today".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .map_err(|_| anyhow!("hourly token aggregates are unavailable"))?
    .0;

    let now = Utc::now();
    let today = now.with_timezone(&Shanghai).date_naive();
    let current_hour_start = shanghai_hour_epoch(today, now.with_timezone(&Shanghai).hour())?;
    let recent_start = current_hour_start - 11 * 3_600;
    let recent_end = current_hour_start + 3_600;
    let token_rows = sqlx::query(
        "SELECT bucket_start_epoch, SUM(total_tokens) AS total_tokens \
         FROM invocation_rollup_hourly \
         WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2 \
         GROUP BY bucket_start_epoch ORDER BY bucket_start_epoch",
    )
    .bind(recent_start)
    .bind(recent_end)
    .fetch_all(&state.pool)
    .await?;
    let mut tokens_by_hour = HashMap::new();
    for row in token_rows {
        let hour = row.try_get::<i64, _>("bucket_start_epoch")?;
        let tokens = row.try_get::<i64, _>("total_tokens")?;
        if tokens < 0 {
            bail!("hourly token aggregate is negative");
        }
        tokens_by_hour.insert(hour, tokens);
    }

    let parallel_rows = sqlx::query(
        "SELECT hour_start_epoch, active_minute_count, parallel_count_sum \
         FROM parallel_work_hourly_rollup \
         WHERE source_scope = 'all' AND hour_start_epoch >= ?1 AND hour_start_epoch < ?2 \
         ORDER BY hour_start_epoch",
    )
    .bind(recent_start)
    .bind(recent_end)
    .fetch_all(&state.pool)
    .await?;
    let mut parallel_by_hour = HashMap::new();
    for row in parallel_rows {
        let hour = row.try_get::<i64, _>("hour_start_epoch")?;
        let active_minutes = row.try_get::<i64, _>("active_minute_count")?;
        let parallel_sum = row.try_get::<i64, _>("parallel_count_sum")?;
        if active_minutes < 0 || parallel_sum < 0 {
            bail!("hourly parallel aggregate is negative");
        }
        let average = if active_minutes == 0 {
            0.0
        } else {
            parallel_sum as f64 / active_minutes as f64
        };
        parallel_by_hour.insert(hour, finite_nonnegative(average)?);
    }

    let mut token_rates = HashMap::new();
    let mut parallel_averages = HashMap::new();
    for index in 0..12 {
        let hour = recent_start + index * 3_600;
        token_rates.insert(
            hour,
            if hour == current_hour_start {
                tokens_per_minute
            } else {
                tokens_by_hour.get(&hour).copied().unwrap_or_default() as f64 / 60.0
            },
        );
        parallel_averages.insert(
            hour,
            if hour == current_hour_start {
                parallel_calls
            } else {
                parallel_by_hour.get(&hour).copied().unwrap_or_default()
            },
        );
    }
    let token_points = build_recent_hour_points(current_hour_start, &token_rates)?;
    let parallel_points = build_recent_hour_points(current_hour_start, &parallel_averages)?;

    let today_points = build_today_token_points(today, now, &timeseries.points)?;
    let yesterday = today
        .pred_opt()
        .ok_or_else(|| anyhow!("Shanghai date underflow"))?;
    let activity_start = yesterday - ChronoDuration::days(89);
    let activity_points =
        load_public_blog_token_activity_90d(&state.pool, activity_start, yesterday)
            .await?
            .into_iter()
            .map(|(date, value)| PublicBlogDailyPoint { date, value })
            .collect();

    let response = PublicBlogRuntimeResponse {
        kind: PUBLIC_BLOG_RUNTIME_KIND,
        tokens_per_minute: PublicBlogStat {
            value: tokens_per_minute,
            trend: PublicBlogTrend {
                range: "recent-hours",
                points: token_points,
            },
        },
        parallel_calls: PublicBlogStat {
            value: parallel_calls,
            trend: PublicBlogTrend {
                range: "recent-hours",
                points: parallel_points,
            },
        },
        today_tokens: PublicBlogStat {
            value: today_tokens,
            trend: PublicBlogTrend {
                range: "today",
                points: today_points,
            },
        },
        token_activity90d: activity_points,
    };
    Ok(Bytes::from(serde_json::to_vec(&response)?))
}

fn build_today_token_points(
    today: NaiveDate,
    now: DateTime<Utc>,
    hourly_points: &[TimeseriesPoint],
) -> Result<Vec<PublicBlogTrendPoint>> {
    let mut hourly_tokens = Vec::with_capacity(hourly_points.len());
    for point in hourly_points {
        let timestamp = DateTime::parse_from_rfc3339(&point.bucket_start)
            .map_err(|_| anyhow!("hourly aggregate timestamp is invalid"))?
            .timestamp();
        if point.total_tokens < 0 {
            bail!("hourly token aggregate is negative");
        }
        hourly_tokens.push((timestamp, point.total_tokens));
    }

    let mut points = Vec::with_capacity(25);
    for hour in 0..=24 {
        let boundary = shanghai_hour_epoch(today, hour)?;
        if boundary <= now.timestamp() {
            let cumulative = hourly_tokens
                .iter()
                .filter(|(start, _)| *start + 3_600 <= boundary)
                .try_fold(0_i64, |sum, (_, value)| sum.checked_add(*value))
                .ok_or_else(|| anyhow!("today token aggregate overflow"))?;
            points.push(PublicBlogTrendPoint {
                timestamp: shanghai_timestamp(boundary)?,
                value: Some(cumulative as f64),
            });
        } else {
            points.push(PublicBlogTrendPoint {
                timestamp: shanghai_timestamp(boundary)?,
                value: None,
            });
        }
    }
    Ok(points)
}

fn build_recent_hour_points(
    current_hour_start: i64,
    values_by_hour: &HashMap<i64, f64>,
) -> Result<Vec<PublicBlogTrendPoint>> {
    let first_hour = current_hour_start - 11 * 3_600;
    (0..12)
        .map(|index| {
            let hour = first_hour + index * 3_600;
            Ok(PublicBlogTrendPoint {
                timestamp: shanghai_timestamp(hour)?,
                value: Some(finite_nonnegative(
                    values_by_hour.get(&hour).copied().unwrap_or_default(),
                )?),
            })
        })
        .collect()
}

fn shanghai_hour_epoch(date: NaiveDate, hour: u32) -> Result<i64> {
    let date = date
        .checked_add_signed(ChronoDuration::days(i64::from(hour) / 24))
        .ok_or_else(|| anyhow!("Shanghai date overflow"))?;
    let local = date
        .and_hms_opt(hour % 24, 0, 0)
        .ok_or_else(|| anyhow!("invalid Shanghai hourly boundary"))?;
    Shanghai
        .from_local_datetime(&local)
        .single()
        .map(|datetime| datetime.timestamp())
        .ok_or_else(|| anyhow!("Shanghai hourly boundary is ambiguous"))
}

fn shanghai_timestamp(epoch: i64) -> Result<String> {
    DateTime::from_timestamp(epoch, 0)
        .map(|datetime| datetime.with_timezone(&Shanghai))
        .map(|datetime| datetime.to_rfc3339_opts(SecondsFormat::Secs, false))
        .ok_or_else(|| anyhow!("timestamp is out of range"))
}

fn finite_nonnegative(value: f64) -> Result<f64> {
    if value.is_finite() && value >= 0.0 {
        Ok(value)
    } else {
        bail!("aggregate value is not finite and non-negative")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn refresh_cache_coalesces_concurrent_requests_and_serves_fresh_snapshot() {
        let cache = Arc::new(PublicBlogRuntimeCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let first_cache = cache.clone();
        let first_calls = calls.clone();
        let second_cache = cache.clone();
        let second_calls = calls.clone();
        let first = tokio::spawn(async move {
            first_cache
                .snapshot_or_refresh(|| async move {
                    first_calls.fetch_add(1, Ordering::SeqCst);
                    sleep(Duration::from_millis(25)).await;
                    Ok(Bytes::from_static(b"snapshot"))
                })
                .await
        });
        let second = tokio::spawn(async move {
            second_cache
                .snapshot_or_refresh(|| async move {
                    second_calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Bytes::from_static(b"other"))
                })
                .await
        });
        let first = first.await.expect("first refresh task").expect("snapshot");
        let second = second
            .await
            .expect("second refresh task")
            .expect("snapshot");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.etag, second.etag);
        assert_eq!(first.body, Bytes::from_static(b"snapshot"));
        let cached_calls = calls.clone();
        let cached = cache
            .snapshot_or_refresh(|| async move {
                cached_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Bytes::from_static(b"unexpected"))
            })
            .await
            .expect("fresh cached snapshot");
        assert_eq!(cached.etag, first.etag);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_failure_returns_last_successful_snapshot() {
        let cache = PublicBlogRuntimeCache::default();
        let successful = cache
            .snapshot_or_refresh(|| async { Ok(Bytes::from_static(b"good")) })
            .await
            .expect("initial snapshot");
        cache.expire_snapshot().await;
        let stale = cache
            .snapshot_or_refresh(|| async { Err(anyhow!("internal failure")) })
            .await
            .expect("last good snapshot");
        assert_eq!(stale.etag, successful.etag);
        assert_eq!(stale.body, Bytes::from_static(b"good"));
    }

    #[tokio::test]
    async fn refresh_timeout_returns_last_successful_snapshot() {
        let cache = PublicBlogRuntimeCache::default();
        let successful = cache
            .snapshot_or_refresh(|| async { Ok(Bytes::from_static(b"good")) })
            .await
            .expect("initial snapshot");
        cache.expire_snapshot().await;
        let refreshed = tokio::time::timeout(
            Duration::from_secs(5),
            cache.snapshot_or_refresh(std::future::pending::<Result<Bytes>>),
        )
        .await
        .expect("refresh has a bounded timeout")
        .expect("stale snapshot after timeout");
        assert_eq!(refreshed.etag, successful.etag);
        assert_eq!(refreshed.body, Bytes::from_static(b"good"));
    }

    #[test]
    fn route_validator_uses_weak_comparison_and_wildcard_matching() {
        let etag = "\"snapshot-hash\"";
        assert!(if_none_match_matches(
            &HeaderValue::from_static("W/\"other\", W/\"snapshot-hash\""),
            etag
        ));
        assert!(if_none_match_matches(&HeaderValue::from_static("*"), etag));
        assert!(!if_none_match_matches(
            &HeaderValue::from_static("\"other\""),
            etag
        ));
    }

    #[test]
    fn rate_limiter_bounds_burst_and_refills_at_configured_rate() {
        let now = Instant::now();
        let mut limiter = PublicBlogRateLimiter::default();
        for _ in 0..PUBLIC_BLOG_RATE_CAPACITY as usize {
            assert_eq!(limiter.take(now), None);
        }
        assert!(limiter.take(now).is_some());
        assert_eq!(limiter.take(now + Duration::from_millis(100)), None);
    }

    #[test]
    fn today_trend_has_25_local_boundaries_and_nulls_future_values() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 28).expect("test date");
        let now = Shanghai
            .with_ymd_and_hms(2026, 9, 28, 10, 30, 0)
            .single()
            .expect("test time")
            .with_timezone(&Utc);
        let points = build_today_token_points(today, now, &[]).expect("today trend");
        assert_eq!(points.len(), 25);
        assert_eq!(points[0].timestamp, "2026-09-28T00:00:00+08:00");
        assert_eq!(points[10].value, Some(0.0));
        assert_eq!(points[11].value, None);
        assert_eq!(points[24].value, None);
    }

    #[test]
    fn serialized_contract_contains_only_the_approved_aggregate_fields() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 28).expect("test date");
        let now = Shanghai
            .with_ymd_and_hms(2026, 9, 28, 10, 30, 0)
            .single()
            .expect("test time")
            .with_timezone(&Utc);
        let response = PublicBlogRuntimeResponse {
            kind: PUBLIC_BLOG_RUNTIME_KIND,
            tokens_per_minute: PublicBlogStat {
                value: 1.0,
                trend: PublicBlogTrend {
                    range: "recent-hours",
                    points: build_recent_hour_points(
                        shanghai_hour_epoch(today, 10).unwrap(),
                        &HashMap::new(),
                    )
                    .unwrap(),
                },
            },
            parallel_calls: PublicBlogStat {
                value: 2.0,
                trend: PublicBlogTrend {
                    range: "recent-hours",
                    points: build_recent_hour_points(
                        shanghai_hour_epoch(today, 10).unwrap(),
                        &HashMap::new(),
                    )
                    .unwrap(),
                },
            },
            today_tokens: PublicBlogStat {
                value: 3.0,
                trend: PublicBlogTrend {
                    range: "today",
                    points: build_today_token_points(today, now, &[]).unwrap(),
                },
            },
            token_activity90d: (0..90)
                .map(|offset| PublicBlogDailyPoint {
                    date: (today - ChronoDuration::days(i64::from(90 - offset))).to_string(),
                    value: i64::from(offset),
                })
                .collect(),
        };
        let value = serde_json::to_value(response).expect("serialize contract");
        let root = value.as_object().expect("root object");
        let root_keys = root.keys().map(String::as_str).collect::<HashSet<_>>();
        assert_eq!(
            root_keys,
            HashSet::from([
                "kind",
                "tokensPerMinute",
                "parallelCalls",
                "todayTokens",
                "tokenActivity90d",
            ])
        );
        for name in ["tokensPerMinute", "parallelCalls", "todayTokens"] {
            let stat = root[name].as_object().expect("stat object");
            assert_eq!(stat.len(), 2);
            assert_eq!(
                stat["trend"]["points"].as_array().unwrap().len(),
                if name == "todayTokens" { 25 } else { 12 }
            );
        }
        assert_eq!(root["tokenActivity90d"].as_array().unwrap().len(), 90);
    }

    #[test]
    fn recent_hour_trend_has_12_ordered_shanghai_boundaries() {
        let current_hour =
            shanghai_hour_epoch(NaiveDate::from_ymd_opt(2026, 9, 28).expect("test date"), 10)
                .expect("current hour");
        let points = build_recent_hour_points(current_hour, &HashMap::new()).expect("recent trend");
        assert_eq!(points.len(), 12);
        assert_eq!(points[0].timestamp, "2026-09-27T23:00:00+08:00");
        assert_eq!(points[11].timestamp, "2026-09-28T10:00:00+08:00");
        assert!(
            points
                .windows(2)
                .all(|pair| pair[0].timestamp < pair[1].timestamp)
        );
    }

    #[tokio::test]
    async fn route_accepts_only_get_and_applies_public_cors_allowlist() {
        let allowed_origins = vec![
            "https://ivanli.cc".to_string(),
            "http://127.0.0.1:12620".to_string(),
        ];
        let router = Router::new()
            .route(
                PUBLIC_BLOG_RUNTIME_PATH,
                any(|request: Request<Body>| async move {
                    reject_non_get(request.method())
                        .unwrap_or_else(|| StatusCode::NO_CONTENT.into_response())
                }),
            )
            .layer(public_blog_runtime_cors_layer(&allowed_origins));

        let get = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(PUBLIC_BLOG_RUNTIME_PATH)
                    .header("origin", "https://ivanli.cc")
                    .body(Body::empty())
                    .expect("GET request"),
            )
            .await
            .expect("GET response");
        assert_eq!(get.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            get.headers().get("access-control-allow-origin"),
            Some(&HeaderValue::from_static("https://ivanli.cc"))
        );
        assert!(
            get.headers()
                .get("access-control-allow-credentials")
                .is_none()
        );

        for method in [Method::POST, Method::HEAD] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(PUBLIC_BLOG_RUNTIME_PATH)
                        .body(Body::empty())
                        .expect("unsupported method request"),
                )
                .await
                .expect("unsupported method response");
            assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
            assert_eq!(
                response.headers().get("allow"),
                Some(&HeaderValue::from_static("GET"))
            );
        }

        let denied = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(PUBLIC_BLOG_RUNTIME_PATH)
                    .header("origin", "https://unlisted.example")
                    .body(Body::empty())
                    .expect("denied-origin request"),
            )
            .await
            .expect("denied-origin response");
        assert!(
            denied
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );

        for (requested_method, allowed) in [("GET", true), ("POST", false)] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::OPTIONS)
                        .uri(PUBLIC_BLOG_RUNTIME_PATH)
                        .header("origin", "http://127.0.0.1:12620")
                        .header("access-control-request-method", requested_method)
                        .header("access-control-request-headers", "if-none-match")
                        .body(Body::empty())
                        .expect("preflight request"),
                )
                .await
                .expect("preflight response");
            let methods = response
                .headers()
                .get("access-control-allow-methods")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            assert_eq!(methods, "GET");
            assert_eq!(methods.contains(requested_method), allowed);
        }
    }

    #[tokio::test]
    async fn endpoint_serializes_aggregate_snapshot_and_honors_etag() {
        let state = crate::tests::test_state_from_config(crate::tests::test_config(), true).await;
        let today = Utc::now().with_timezone(&Shanghai).date_naive();
        let end = today.pred_opt().expect("yesterday");
        let start = end - ChronoDuration::days(89);
        sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = ?2 WHERE id = ?3",
        )
        .bind("ready")
        .bind(start.to_string())
        .bind(1_i64)
        .execute(&state.pool)
        .await
        .expect("set long-term fixture ready");
        for offset in 0..90_i64 {
            let date = start + ChronoDuration::days(offset);
            sqlx::query(
                "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, token_total, token_samples) VALUES (?1, 'overall', 'all', 'Overall', ?2, 1)",
            )
            .bind(date.to_string())
            .bind(offset)
            .execute(&state.pool)
            .await
            .expect("insert daily fixture");
        }

        let router = build_public_blog_runtime_router(state.clone());
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(PUBLIC_BLOG_RUNTIME_PATH)
                    .body(Body::empty())
                    .expect("snapshot request"),
            )
            .await
            .expect("snapshot response");
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response.headers().get("etag").cloned().expect("ETag");
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("snapshot body");
        let value: Value = serde_json::from_slice(&body).expect("snapshot JSON");
        let root = value.as_object().expect("snapshot object");
        assert_eq!(root["kind"], PUBLIC_BLOG_RUNTIME_KIND);
        assert_eq!(
            root["tokensPerMinute"]["trend"]["points"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        assert_eq!(
            root["parallelCalls"]["trend"]["points"]
                .as_array()
                .unwrap()
                .len(),
            12
        );
        assert_eq!(
            root["todayTokens"]["trend"]["points"]
                .as_array()
                .unwrap()
                .len(),
            25
        );
        let activity = root["tokenActivity90d"].as_array().expect("daily points");
        assert_eq!(activity.len(), 90);
        assert_eq!(activity.first().unwrap()["date"], start.to_string());
        assert_eq!(activity.last().unwrap()["date"], end.to_string());

        let not_modified = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(PUBLIC_BLOG_RUNTIME_PATH)
                    .header("if-none-match", etag)
                    .body(Body::empty())
                    .expect("conditional request"),
            )
            .await
            .expect("conditional response");
        assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
        state.shutdown.cancel();
    }
}
