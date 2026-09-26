use super::*;
use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use chrono_tz::Asia::Shanghai;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, QueryBuilder, Sqlite};
use std::collections::{BTreeMap, HashSet};

const INVOCATION_TIMELINE_MAX_RECORDS: i64 = 2_000;
const INVOCATION_TIMELINE_MAX_DURATION_MS: f64 = 30.0 * 24.0 * 60.0 * 60.0 * 1_000.0;

#[derive(Debug, FromRow)]
struct TimelineAccountFallbackRow {
    invoke_id: String,
    occurred_at: String,
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTimelineQuery {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTimelineRecord {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) end_at: Option<String>,
    pub(crate) is_in_flight: bool,
    pub(crate) status: Option<String>,
    pub(crate) live_phase: Option<String>,
    pub(crate) first_token_ms: Option<f64>,
    pub(crate) t_total_ms: Option<f64>,
    pub(crate) pool_attempt_count: Option<i64>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) upstream_account_name: Option<String>,
    pub(crate) failure_class: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InvocationTimelineResponse {
    pub(crate) range_start: String,
    pub(crate) range_end: String,
    pub(crate) as_of: String,
    pub(crate) total: i64,
    pub(crate) over_limit: bool,
    pub(crate) records: Vec<InvocationTimelineRecord>,
}

pub(crate) async fn fetch_timeline(
    State(state): State<Arc<AppState>>,
    Query(params): Query<InvocationTimelineQuery>,
) -> Result<Json<InvocationTimelineResponse>, ApiError> {
    let parse_bound = |value: &str, field: &str| {
        DateTime::parse_from_rfc3339(value.trim())
            .with_context(|| format!("invalid {field}: {value}"))
            .map(|value| value.with_timezone(&Utc))
            .map_err(ApiError::bad_request)
    };
    let range_start = parse_bound(&params.from, "from")?;
    let range_end = parse_bound(&params.to, "to")?;
    if range_start >= range_end {
        return Err(ApiError::bad_request(anyhow!("from must be before to")));
    }
    if params
        .upstream_account_id
        .is_some_and(|account_id| account_id <= 0)
    {
        return Err(ApiError::bad_request(anyhow!("invalid upstream account")));
    }

    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let filters = build_invocation_filters(&ListQuery {
        upstream_account_id: params.upstream_account_id,
        ..ListQuery::default()
    })?;
    let start_bound = crate::db_occurred_at_lower_bound(range_start);
    let overlap_start =
        range_start - chrono::Duration::milliseconds(INVOCATION_TIMELINE_MAX_DURATION_MS as i64);
    let overlap_start_bound = crate::db_occurred_at_lower_bound(overlap_start);
    let end_bound = crate::db_occurred_at_upper_bound(range_end);
    let mut query = build_invocation_select_query();
    let mut persisted_filters = filters.clone();
    persisted_filters.upstream_account_id = None;
    apply_invocation_records_filters(&mut query, &persisted_filters, source_scope, None);
    if let Some(upstream_account_id) = params.upstream_account_id {
        query
            .push(" AND ")
            .push(timeline_upstream_account_id_sql("codex_invocations"))
            .push(" = ")
            .push_bind(upstream_account_id);
    }
    query
        .push(" AND occurred_at < ")
        .push_bind(end_bound.clone())
        .push(" AND occurred_at >= ")
        .push_bind(overlap_start_bound)
        .push(" AND (occurred_at >= ")
        .push_bind(start_bound.clone())
        .push(" OR (t_total_ms IS NOT NULL AND t_total_ms >= 0 AND t_total_ms <= ")
        .push_bind(INVOCATION_TIMELINE_MAX_DURATION_MS)
        .push(" AND julianday(occurred_at) + t_total_ms / 86400000.0 >= julianday(")
        .push_bind(start_bound.clone())
        .push(")")
        .push(")")
        .push(" OR LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')")
        .push(") ORDER BY occurred_at ASC, id ASC LIMIT ")
        .push_bind(INVOCATION_TIMELINE_MAX_RECORDS + 1);
    let mut records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await?;
    for record in &mut records {
        hydrate_api_invocation_blocked_binding(record);
    }

    let include_runtime_records = timeline_window_includes_current_day(range_start, range_end);
    let mut runtime_records = if include_runtime_records {
        runtime_overlay_snapshot(state.as_ref())
    } else {
        Vec::new()
    };
    runtime_records.retain(|record| {
        if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
            return false;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            return false;
        };
        occurred_at < range_end
            && (runtime_record_is_in_flight(record)
                || timeline_record_overlaps(occurred_at, record.t_total_ms, range_start, range_end))
    });
    let skip_account_hydration = records.len() > INVOCATION_TIMELINE_MAX_RECORDS as usize;
    if !skip_account_hydration {
        hydrate_timeline_accounts(&state.pool, &mut records, source_scope).await?;
        hydrate_timeline_accounts(&state.pool, &mut runtime_records, source_scope).await?;
    }
    let terminal_runtime_keys = if runtime_records.is_empty() {
        HashSet::new()
    } else {
        query_terminal_db_keys_for_runtime_records(&state.pool, &runtime_records, None).await?
    };
    let mut runtime_keys = HashSet::new();
    let mut merged = records
        .into_iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<BTreeMap<_, _>>();
    for record in runtime_records {
        let key = (record.invoke_id.clone(), record.occurred_at.clone());
        if terminal_runtime_keys.contains(&key)
            || !runtime_record_matches_filters(&record, &filters, source_scope)
        {
            continue;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let overlaps = runtime_record_is_in_flight(&record)
            || timeline_record_overlaps(occurred_at, record.t_total_ms, range_start, range_end);
        if occurred_at < range_end && overlaps {
            runtime_keys.insert(key.clone());
            merged.insert(key, record);
        }
    }

    let total = merged.len() as i64;
    let over_limit = total > INVOCATION_TIMELINE_MAX_RECORDS;
    let timeline_records = if over_limit {
        Vec::new()
    } else {
        merged
            .into_values()
            .map(|record| {
                let is_in_flight = runtime_record_is_in_flight(&record);
                let total_ms = valid_timeline_duration_ms(record.t_total_ms);
                let end_at = if is_in_flight {
                    None
                } else {
                    total_ms.and_then(|duration| {
                        parse_to_utc_datetime(&record.occurred_at)
                            .and_then(|start| timeline_end_at(start, Some(duration)))
                    })
                };
                let occurred_at = record.occurred_at.clone();
                let status = invocation_display_status_value(&record).map(str::to_string);
                let is_runtime =
                    runtime_keys.contains(&(record.invoke_id.clone(), occurred_at.clone()));
                InvocationTimelineRecord {
                    id: record.id,
                    invoke_id: record.invoke_id.clone(),
                    occurred_at: parse_to_utc_datetime(&occurred_at)
                        .map(format_utc_iso_precise)
                        .unwrap_or(occurred_at),
                    end_at,
                    is_in_flight,
                    status,
                    live_phase: if is_runtime {
                        runtime_record_live_phase(&record).map(str::to_string)
                    } else {
                        record.live_phase.clone()
                    },
                    first_token_ms: if is_runtime {
                        runtime_record_first_token_ms(&record)
                    } else {
                        record
                            .first_token_ms
                            .filter(|value| value.is_finite() && *value >= 0.0)
                    },
                    t_total_ms: total_ms,
                    pool_attempt_count: record.pool_attempt_count,
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name,
                    failure_class: record.failure_class,
                }
            })
            .collect()
    };

    Ok(Json(InvocationTimelineResponse {
        range_start: format_utc_iso(range_start),
        range_end: format_utc_iso(range_end),
        as_of: format_utc_iso_precise(Utc::now()),
        total,
        over_limit,
        records: timeline_records,
    }))
}

fn timeline_record_overlaps(
    occurred_at: DateTime<Utc>,
    total_ms: Option<f64>,
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> bool {
    if occurred_at >= range_end {
        return false;
    }
    match valid_timeline_duration_ms(total_ms) {
        Some(duration) => timeline_end_at(occurred_at, Some(duration))
            .and_then(|value| parse_to_utc_datetime(&value))
            .is_some_and(|end| end >= range_start),
        None => occurred_at >= range_start,
    }
}

fn timeline_window_includes_current_day(
    range_start: DateTime<Utc>,
    range_end: DateTime<Utc>,
) -> bool {
    let now = Utc::now();
    let current_day_start = now
        .with_timezone(&Shanghai)
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| Shanghai.from_local_datetime(&midnight).single())
        .map(|value| value.with_timezone(&Utc));
    let Some(current_day_start) = current_day_start else {
        return false;
    };
    let next_day_start = current_day_start + chrono::Duration::days(1);
    range_end > current_day_start && range_start < next_day_start
}

fn valid_timeline_duration_ms(value: Option<f64>) -> Option<f64> {
    value.filter(|value| {
        value.is_finite() && *value >= 0.0 && *value <= INVOCATION_TIMELINE_MAX_DURATION_MS
    })
}

fn timeline_end_at(start: DateTime<Utc>, duration: Option<f64>) -> Option<String> {
    let milliseconds = valid_timeline_duration_ms(duration)?.round();
    if milliseconds > i64::MAX as f64 {
        return None;
    }
    start
        .checked_add_signed(chrono::Duration::milliseconds(milliseconds as i64))
        .map(format_utc_iso_precise)
}

async fn hydrate_timeline_accounts(
    pool: &Pool<Sqlite>,
    records: &mut [ApiInvocation],
    source_scope: InvocationSourceScope,
) -> Result<(), ApiError> {
    for record in records.iter_mut() {
        if record
            .upstream_account_id
            .is_some_and(|account_id| account_id <= 0)
        {
            record.upstream_account_id = None;
        }
    }
    let keys = records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>();
    if keys.is_empty() {
        return Ok(());
    }
    let resolved_id = timeline_upstream_account_id_sql("codex_invocations");
    let mut rows = Vec::new();
    let key_list = keys.into_iter().collect::<Vec<_>>();
    for chunk in key_list.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT codex_invocations.invoke_id, codex_invocations.occurred_at, ",
        );
        query
            .push(resolved_id.as_str())
            .push(" AS upstream_account_id, ")
            .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
            .push(" AS upstream_account_name FROM codex_invocations WHERE ");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(codex_invocations.invoke_id = ")
                .push_bind(invoke_id.as_str())
                .push(" AND codex_invocations.occurred_at = ")
                .push_bind(occurred_at.as_str())
                .push(")");
        }
        if source_scope == InvocationSourceScope::ProxyOnly {
            query
                .push(" AND codex_invocations.source = ")
                .push_bind(SOURCE_PROXY);
        }
        rows.extend(
            query
                .build_query_as::<TimelineAccountFallbackRow>()
                .fetch_all(pool)
                .await?,
        );
    }
    let fallbacks = rows
        .into_iter()
        .map(|row| ((row.invoke_id.clone(), row.occurred_at.clone()), row))
        .collect::<std::collections::HashMap<_, _>>();
    for record in records {
        if let Some(row) = fallbacks.get(&(record.invoke_id.clone(), record.occurred_at.clone())) {
            record.upstream_account_id = row.upstream_account_id;
            record.upstream_account_name = row.upstream_account_name.clone();
        }
    }
    Ok(())
}

fn timeline_upstream_account_id_sql(invocation_ref: &str) -> String {
    let payload_is_valid = format!("json_valid({invocation_ref}.payload)");
    let value = format!(
        "CASE WHEN {payload_is_valid} THEN json_extract({invocation_ref}.payload, '$.upstreamAccountId') END"
    );
    let value_type = format!(
        "CASE WHEN {payload_is_valid} THEN json_type({invocation_ref}.payload, '$.upstreamAccountId') END"
    );
    let trimmed = format!("TRIM({value})");
    let normalized = format!("ltrim({trimmed}, '0')");
    let valid_payload_id = format!(
        "({value} IS NOT NULL AND (({value_type} = 'integer' AND typeof({value}) = 'integer' AND {value} > 0) OR ({value_type} = 'real' AND {value} > 0 AND {value} < 9223372036854775807.0 AND CAST({value} AS REAL) = CAST({value} AS INTEGER)) OR ({value_type} = 'text' AND {trimmed} <> '' AND {trimmed} NOT GLOB '*[^0-9]*' AND {normalized} <> '' AND (length({normalized}) < 19 OR (length({normalized}) = 19 AND {normalized} <= '9223372036854775807')))))"
    );
    format!(
        "COALESCE(CASE WHEN {payload_is_valid} AND {valid_payload_id} THEN CAST({value} AS INTEGER) END, (SELECT attempt.upstream_account_id FROM pool_upstream_request_attempts attempt WHERE attempt.invoke_id = {invocation_ref}.invoke_id AND attempt.occurred_at = {invocation_ref}.occurred_at AND attempt.upstream_account_id IS NOT NULL AND attempt.upstream_account_id > 0 ORDER BY attempt.attempt_index DESC, attempt.id DESC LIMIT 1))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("valid test timestamp")
    }

    #[test]
    fn cross_midnight_window_includes_only_overlapping_invocations() {
        let range_start = at(86_400);
        let range_end = at(90_000);
        assert!(timeline_record_overlaps(
            at(86_399),
            Some(2_000.0),
            range_start,
            range_end
        ));
        assert!(!timeline_record_overlaps(
            at(86_399),
            Some(500.0),
            range_start,
            range_end
        ));
        assert!(timeline_record_overlaps(
            at(86_401),
            None,
            range_start,
            range_end
        ));
        assert!(!timeline_record_overlaps(
            at(90_000),
            Some(1_000.0),
            range_start,
            range_end
        ));
    }

    #[test]
    fn invalid_total_duration_is_treated_as_unknown_and_stays_in_window() {
        let range_start = at(100);
        let range_end = at(200);
        assert!(timeline_record_overlaps(
            at(150),
            Some(f64::NAN),
            range_start,
            range_end
        ));
        assert!(timeline_record_overlaps(
            at(150),
            Some(-1.0),
            range_start,
            range_end
        ));
        assert!(timeline_record_overlaps(
            at(150),
            Some(INVOCATION_TIMELINE_MAX_DURATION_MS + 1.0),
            range_start,
            range_end
        ));
        assert!(!timeline_record_overlaps(
            at(99),
            Some(f64::NAN),
            range_start,
            range_end
        ));
    }

    #[test]
    fn runtime_overlay_is_limited_to_windows_that_intersect_the_current_day() {
        let now = Utc::now();
        let current_day_start = now
            .with_timezone(&Shanghai)
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|midnight| Shanghai.from_local_datetime(&midnight).single())
            .map(|value| value.with_timezone(&Utc))
            .expect("current day midnight");

        assert!(!timeline_window_includes_current_day(
            current_day_start - chrono::Duration::days(1),
            current_day_start,
        ));
        assert!(timeline_window_includes_current_day(
            current_day_start,
            current_day_start + chrono::Duration::hours(1),
        ));
        assert!(timeline_window_includes_current_day(
            current_day_start - chrono::Duration::minutes(5),
            current_day_start + chrono::Duration::minutes(5),
        ));
    }

    #[tokio::test]
    async fn endpoint_returns_cross_window_overlap_once_with_account_filter() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let range_start = at(86_400);
        let range_end = at(90_000);
        for (invoke_id, occurred_at, total_ms, account_id) in [
            ("cross", at(86_399), 2_000_i64, 42_i64),
            ("outside", at(86_399), 500_i64, 42_i64),
            ("inside", at(86_401), 100_i64, 99_i64),
        ] {
            sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES (?1, ?2, 'proxy', 'success', ?3, ?4, '', 'full')",
            )
            .bind(invoke_id)
            .bind(db_occurred_at_lower_bound(occurred_at))
            .bind(total_ms)
            .bind(format!("{{\"upstreamAccountId\":{account_id}}}"))
            .execute(&state.pool)
            .await
            .expect("insert timeline fixture");
        }
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, payload, raw_response, detail_level) VALUES ('live-cross', ?1, 'proxy', 'running', ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_399)))
        .bind(r#"{"upstreamAccountId":42}"#)
        .execute(&state.pool)
        .await
        .expect("insert live timeline fixture");
        let Json(response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(range_start),
                to: format_utc_iso(range_end),
                upstream_account_id: Some(42),
            }),
        )
        .await
        .expect("fetch timeline fixture");
        assert!(!response.over_limit);
        assert_eq!(response.total, 2);
        assert_eq!(response.records.len(), 2);
        assert_eq!(response.records[0].invoke_id, "cross");
        assert_eq!(response.records[0].t_total_ms, Some(2_000.0));
        assert_eq!(response.records[1].invoke_id, "live-cross");
        assert!(response.records[1].is_in_flight);
        assert_eq!(response.records[1].end_at, None);
        state.pool.close().await;
    }

    #[tokio::test]
    async fn account_filter_rejects_overflow_payload_and_uses_attempt_fallback() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let occurred_at = db_occurred_at_lower_bound(at(86_400));
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('overflow', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(&occurred_at)
        .bind(r#"{"upstreamAccountId":"9223372036854775808"}"#)
        .execute(&state.pool)
        .await
        .expect("insert overflow timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (1, 'overflow', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 42)",
        )
        .bind(&occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('overflow-integer', ?1, 'proxy', 'success', 100, ?2, '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_500)))
        .bind(r#"{"upstreamAccountId":9223372036854775808}"#)
        .execute(&state.pool)
        .await
        .expect("insert integer overflow timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (2, 'overflow-integer', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 43)",
        )
        .bind(db_occurred_at_lower_bound(at(86_500)))
        .execute(&state.pool)
            .await
            .expect("insert integer attempt fallback fixture");
        sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, t_total_ms, payload, raw_response, detail_level) VALUES ('malformed', ?1, 'proxy', 'success', 100, '{bad-json', '', 'full')",
        )
        .bind(db_occurred_at_lower_bound(at(86_600)))
        .execute(&state.pool)
        .await
        .expect("insert malformed timeline fixture");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, attempt_index, distinct_account_index, same_account_retry_index, status, upstream_account_id) VALUES (3, 'malformed', ?1, '/v1/responses', 'pool', 1, 1, 0, 'success', 44)",
        )
        .bind(db_occurred_at_lower_bound(at(86_600)))
        .execute(&state.pool)
        .await
        .expect("insert malformed attempt fallback fixture");

        let Json(response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(42),
            }),
        )
        .await
        .expect("fetch overflow timeline fixture");
        assert_eq!(response.total, 1);
        assert_eq!(response.records[0].invoke_id, "overflow");
        assert_eq!(response.records[0].upstream_account_id, Some(42));
        let Json(integer_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(43),
            }),
        )
        .await
        .expect("fetch integer overflow timeline fixture");
        assert_eq!(integer_response.total, 1);
        assert_eq!(integer_response.records[0].invoke_id, "overflow-integer");
        assert_eq!(integer_response.records[0].upstream_account_id, Some(43));
        let Json(malformed_response) = fetch_timeline(
            State(state.clone()),
            Query(InvocationTimelineQuery {
                from: format_utc_iso(at(86_000)),
                to: format_utc_iso(at(87_000)),
                upstream_account_id: Some(44),
            }),
        )
        .await
        .expect("fetch malformed timeline fixture");
        assert_eq!(malformed_response.total, 1);
        assert_eq!(malformed_response.records[0].invoke_id, "malformed");
        assert_eq!(malformed_response.records[0].upstream_account_id, Some(44));
        state.pool.close().await;
    }
}
