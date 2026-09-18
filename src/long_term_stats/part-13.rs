fn build_daily_points(
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

fn normalize_long_term_model(row: &LongTermInvocationRow) -> String {
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

fn normalize_long_term_reasoning(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未指定")
        .to_string()
}

fn long_term_model_series_key(model: &str, reasoning: &str) -> String {
    let payload =
        serde_json::to_vec(&[model, reasoning]).expect("model series key payload is serializable");
    format!(
        "model:v2:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    )
}

fn normalize_long_term_upstream(row: &LongTermInvocationRow) -> (String, String) {
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

fn is_success_status(status: Option<&str>, error_message: Option<&str>) -> bool {
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
struct LongTermTimestamp {
    epoch_ms: i64,
    sub_millisecond_nanos: u32,
}

fn parse_long_term_timestamp(raw: &str) -> Option<LongTermTimestamp> {
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

fn parse_long_term_timestamp_ms(raw: &str) -> Option<i64> {
    parse_long_term_timestamp(raw).map(|timestamp| timestamp.epoch_ms)
}

fn long_term_interval_end_ms(start: LongTermTimestamp, duration_ms: f64) -> Option<i64> {
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

fn union_interval_duration(intervals: &[(i64, i64)]) -> i64 {
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

fn internal_error_tuple(error: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}
