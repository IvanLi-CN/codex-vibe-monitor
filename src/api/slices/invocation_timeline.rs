use super::*;
use anyhow::{Context, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const INVOCATION_TIMELINE_MAX_RECORDS: i64 = 2_000;

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

    let source_scope = resolve_default_source_scope(&state.pool).await?;
    let filters = build_invocation_filters(&ListQuery {
        upstream_account_id: params.upstream_account_id,
        ..ListQuery::default()
    })?;
    let start_bound = crate::db_occurred_at_lower_bound(range_start);
    let end_bound = crate::db_occurred_at_upper_bound(range_end);
    let mut query = build_invocation_select_query();
    apply_invocation_records_filters(&mut query, &filters, source_scope, None);
    query
        .push(" AND occurred_at < ")
        .push_bind(end_bound.clone())
        .push(" AND (occurred_at >= ")
        .push_bind(start_bound.clone())
        .push(" OR (t_total_ms IS NOT NULL AND t_total_ms >= 0 AND ")
        .push("julianday(occurred_at) + t_total_ms / 86400000.0 >= julianday(")
        .push_bind(start_bound)
        .push("))) ORDER BY occurred_at ASC, id ASC LIMIT ")
        .push_bind(INVOCATION_TIMELINE_MAX_RECORDS + 1);
    let mut records = query
        .build_query_as::<ApiInvocation>()
        .fetch_all(&state.pool)
        .await?;
    for record in &mut records {
        hydrate_api_invocation_blocked_binding(record);
    }

    let runtime_records = runtime_overlay_snapshot(state.as_ref());
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
        if params.upstream_account_id.is_some()
            && record.upstream_account_id != params.upstream_account_id
        {
            continue;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&record.occurred_at) else {
            continue;
        };
        let overlaps = runtime_record_is_in_flight(&record)
            || timeline_record_overlaps(occurred_at, record.t_total_ms, range_start, range_end);
        if occurred_at < range_end && overlaps {
            merged.insert(
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            );
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
                let total_ms = record
                    .t_total_ms
                    .filter(|value| value.is_finite() && *value >= 0.0);
                let end_at = if is_in_flight {
                    None
                } else {
                    total_ms.and_then(|duration| {
                        parse_to_utc_datetime(&record.occurred_at).map(|start| {
                            format_utc_iso_precise(
                                start + chrono::Duration::milliseconds(duration.round() as i64),
                            )
                        })
                    })
                };
                let occurred_at = record.occurred_at.clone();
                let status = invocation_display_status_value(&record).map(str::to_string);
                InvocationTimelineRecord {
                    id: record.id,
                    invoke_id: record.invoke_id.clone(),
                    occurred_at: parse_to_utc_datetime(&occurred_at)
                        .map(format_utc_iso_precise)
                        .unwrap_or(occurred_at),
                    end_at,
                    is_in_flight,
                    status,
                    live_phase: record.live_phase,
                    first_token_ms: record
                        .first_token_ms
                        .filter(|value| value.is_finite() && *value >= 0.0),
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
    match total_ms.filter(|value| value.is_finite() && *value >= 0.0) {
        Some(duration) => {
            occurred_at + chrono::Duration::milliseconds(duration.round() as i64) >= range_start
        }
        None => occurred_at >= range_start,
    }
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
        assert!(!timeline_record_overlaps(
            at(99),
            Some(f64::NAN),
            range_start,
            range_end
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
        assert_eq!(response.total, 1);
        assert_eq!(response.records.len(), 1);
        assert_eq!(response.records[0].invoke_id, "cross");
        assert_eq!(response.records[0].t_total_ms, Some(2_000.0));
        state.pool.close().await;
    }
}
