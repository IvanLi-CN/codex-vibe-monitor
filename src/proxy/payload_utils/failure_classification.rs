use super::*;
#[derive(Debug, FromRow)]
pub(crate) struct InvocationServiceTierBackfillCandidate {
    id: i64,
    source: String,
    raw_response: String,
    response_raw_path: Option<String>,
    current_service_tier: Option<String>,
}

pub(crate) async fn backfill_invocation_service_tiers_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<InvocationServiceTierBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = InvocationServiceTierBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, InvocationServiceTierBackfillCandidate>(
            INVOCATION_SERVICE_TIER_BACKFILL_QUERY,
        )
        .bind(last_seen_id)
        .bind(SOURCE_PROXY)
        .bind(SERVICE_TIER_STREAM_BACKFILL_VERSION)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;
            let Some(service_tier) = resolve_invocation_service_tier(
                &candidate,
                raw_path_fallback_root,
                &mut summary,
                &mut samples,
            )
            .await
            else {
                continue;
            };
            let mark_stream_backfill = candidate.source == SOURCE_PROXY;
            if candidate
                .current_service_tier
                .as_deref()
                .and_then(normalize_service_tier)
                .is_some_and(|current| current == service_tier)
                && !mark_stream_backfill
            {
                continue;
            }
            summary.updated += update_invocation_service_tier(
                pool,
                candidate.id,
                &service_tier,
                mark_stream_backfill,
            )
            .await?;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

const INVOCATION_SERVICE_TIER_BACKFILL_QUERY: &str = r#"
            SELECT
                id,
                source,
                raw_response,
                response_raw_path,
                CASE
                  WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'
                    THEN json_extract(payload, '$.serviceTier')
                  WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'
                    THEN json_extract(payload, '$.service_tier')
                END AS current_service_tier
            FROM codex_invocations
            WHERE id > ?1
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) IS NULL
                OR TRIM(CAST(COALESCE(json_extract(payload, '$.serviceTier'), json_extract(payload, '$.service_tier')) AS TEXT)) = ''
                OR (
                    source = ?2
                    AND COALESCE(
                        CASE
                          WHEN json_valid(payload) AND json_type(payload, '$.serviceTierBackfillVersion') = 'text'
                            THEN json_extract(payload, '$.serviceTierBackfillVersion')
                          WHEN json_valid(payload) AND json_type(payload, '$.service_tier_backfill_version') = 'text'
                            THEN json_extract(payload, '$.service_tier_backfill_version')
                        END,
                        ''
                    ) != ?3
                    AND (
                        response_raw_path IS NOT NULL
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'service_tier') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'servicetier') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.completed') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.failed') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.created') > 0
                        OR INSTR(LOWER(COALESCE(raw_response, '')), 'response.in_progress') > 0
                    )
                )
              )
            ORDER BY id ASC
            LIMIT ?4
            "#;

async fn resolve_invocation_service_tier(
    candidate: &InvocationServiceTierBackfillCandidate,
    raw_path_fallback_root: Option<&Path>,
    summary: &mut InvocationServiceTierBackfillSummary,
    samples: &mut Vec<String>,
) -> Option<String> {
    let mut service_tier = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        candidate.raw_response.as_bytes(),
        false,
        None,
    )
    .service_tier;
    if service_tier.is_none()
        && candidate.source == SOURCE_PROXY
        && let Some(path) = candidate.response_raw_path.as_deref()
    {
        match read_proxy_raw_bytes(path, raw_path_fallback_root) {
            Ok(bytes) => {
                let (payload_for_parse, _) = decode_response_payload_for_usage(&bytes, None);
                service_tier = parse_target_response_payload(
                    ProxyCaptureTarget::Responses,
                    payload_for_parse.as_ref(),
                    false,
                    None,
                )
                .service_tier;
            }
            Err(_) => {
                summary.skipped_missing_file += 1;
                push_backfill_sample(
                    samples,
                    format!(
                        "id={} response_raw_path={} reason=missing_file",
                        candidate.id, path
                    ),
                );
                return None;
            }
        }
    }
    match service_tier {
        Some(value) => Some(value),
        None => {
            summary.skipped_missing_tier += 1;
            None
        }
    }
}

async fn update_invocation_service_tier(
    pool: &Pool<Sqlite>,
    id: i64,
    service_tier: &str,
    mark_stream_backfill: bool,
) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE codex_invocations SET payload = CASE WHEN ?3 IS NULL THEN json_set(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END, '$.serviceTier', ?1) ELSE json_set(json_set(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END, '$.serviceTier', ?1), '$.serviceTierBackfillVersion', ?3) END WHERE id = ?2",
    )
    .bind(service_tier)
    .bind(id)
    .bind(mark_stream_backfill.then_some(SERVICE_TIER_STREAM_BACKFILL_VERSION))
    .execute(pool)
    .await?
    .rows_affected())
}

#[cfg(test)]
pub(crate) async fn backfill_invocation_service_tiers(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<InvocationServiceTierBackfillSummary> {
    Ok(
        backfill_invocation_service_tiers_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

#[derive(Debug, FromRow)]
pub(crate) struct FailureClassificationBackfillRow {
    id: i64,
    source: String,
    status: Option<String>,
    error_message: Option<String>,
    failure_kind: Option<String>,
    failure_class: Option<String>,
    is_actionable: Option<i64>,
    payload: Option<String>,
    raw_response: String,
    response_raw_path: Option<String>,
}

pub(crate) fn parse_proxy_response_capture_from_stored_bytes(
    target: ProxyCaptureTarget,
    bytes: &[u8],
    is_stream: bool,
) -> ResponseCaptureInfo {
    let (payload_for_parse, _) = decode_response_payload_for_usage(bytes, None);
    parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None)
}

pub(crate) fn format_upstream_response_failed_message(
    response_info: &ResponseCaptureInfo,
) -> String {
    let upstream_message = response_info
        .upstream_error_message
        .as_deref()
        .unwrap_or("upstream response failed");
    if let Some(code) = response_info.upstream_error_code.as_deref() {
        format!(
            "[{}] {}: {}",
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED, code, upstream_message
        )
    } else {
        format!(
            "[{}] {}",
            PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED, upstream_message
        )
    }
}

pub(crate) fn update_proxy_payload_failure_details(
    payload: Option<&str>,
    failure_kind: Option<&str>,
    response_info: &ResponseCaptureInfo,
) -> String {
    let mut value = payload
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .filter(|value| value.is_object())
        .unwrap_or_else(|| json!({}));
    let object = value
        .as_object_mut()
        .expect("payload summary must be an object");

    object.insert(
        "failureKind".to_string(),
        failure_kind
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "streamTerminalEvent".to_string(),
        response_info
            .stream_terminal_event
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamErrorCode".to_string(),
        response_info
            .upstream_error_code
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamErrorMessage".to_string(),
        response_info
            .upstream_error_message
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "upstreamRequestId".to_string(),
        response_info
            .upstream_request_id
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );
    object.insert(
        "usageMissingReason".to_string(),
        response_info
            .usage_missing_reason
            .as_ref()
            .map(|value| Value::String(value.clone()))
            .unwrap_or(Value::Null),
    );

    serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string())
}

pub(crate) fn should_upgrade_to_upstream_response_failed(
    row: &FailureClassificationBackfillRow,
    existing_kind: Option<&str>,
) -> bool {
    if matches!(
        existing_kind,
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
            | Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
            | Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM)
            | Some(PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT)
            | Some(PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT)
            | Some(PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED)
    ) {
        return false;
    }

    crate::maintenance::invocation_status_is_success_like(
        row.status.as_deref(),
        row.error_message.as_deref(),
    ) || existing_kind.is_none()
        || existing_kind == Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
}

pub(crate) fn parse_proxy_response_failure_from_persisted_record(
    row: &FailureClassificationBackfillRow,
    raw_path_fallback_root: Option<&Path>,
) -> Result<Option<ResponseCaptureInfo>> {
    if row.source != SOURCE_PROXY {
        return Ok(None);
    }

    let (target, is_stream) = parse_proxy_capture_summary(row.payload.as_deref());
    let preview_info = parse_proxy_response_capture_from_stored_bytes(
        target,
        row.raw_response.as_bytes(),
        is_stream,
    );
    let preview_has_failure = preview_info.stream_terminal_event.is_some();
    let preview_is_complete = preview_has_failure
        && preview_info.upstream_error_message.is_some()
        && preview_info.upstream_request_id.is_some();

    if preview_is_complete || row.response_raw_path.is_none() {
        return Ok(preview_has_failure.then_some(preview_info));
    }

    let Some(path) = row.response_raw_path.as_deref() else {
        return Ok(preview_has_failure.then_some(preview_info));
    };

    match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(bytes) => {
            let full_info =
                parse_proxy_response_capture_from_stored_bytes(target, &bytes, is_stream);
            if full_info.stream_terminal_event.is_some() {
                Ok(Some(full_info))
            } else {
                Ok(preview_has_failure.then_some(preview_info))
            }
        }
        Err(_err) if preview_has_failure => Ok(Some(preview_info)),
        Err(err) => Err(err.into()),
    }
}

pub(crate) async fn backfill_failure_classification_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<FailureClassificationBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = FailureClassificationBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let rows = sqlx::query_as::<_, FailureClassificationBackfillRow>(
            FAILURE_CLASSIFICATION_BACKFILL_QUERY,
        )
        .bind(last_seen_id)
        .bind(SOURCE_PROXY)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        if let Some(last) = rows.last() {
            last_seen_id = last.id;
        }
        summary.scanned += rows.len() as u64;

        let mut tx = pool.begin().await?;
        let mut updated_ids = Vec::new();
        for row in rows {
            let outcome = process_failure_classification_row(
                &mut tx,
                &row,
                raw_path_fallback_root,
                &mut samples,
            )
            .await?;
            summary.updated += outcome.updated;
            if let Some(id) = outcome.updated_id {
                updated_ids.push(id);
            }
        }
        if !updated_ids.is_empty() {
            recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
        }
        tx.commit().await?;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

const FAILURE_CLASSIFICATION_BACKFILL_QUERY: &str = r#"
            SELECT
                id,
                source,
                status,
                error_message,
                failure_kind,
                failure_class,
                is_actionable,
                payload,
                raw_response,
                response_raw_path
            FROM codex_invocations
            WHERE id > ?1
              AND (
                failure_class IS NULL
                OR TRIM(COALESCE(failure_class, '')) = ''
                OR is_actionable IS NULL
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) NOT IN ('success', 'warning_success')
                    AND TRIM(COALESCE(status, '')) != ''
                    AND TRIM(COALESCE(failure_kind, '')) = ''
                )
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) NOT IN ('success', 'warning_success')
                    AND TRIM(COALESCE(failure_class, '')) = 'none'
                )
                OR (
                    source = ?2
                    AND LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'warning_success')
                    AND (
                        raw_response LIKE '%response.failed%'
                        OR raw_response LIKE '%"type":"error"%'
                        OR (
                            json_valid(payload)
                            AND (
                                TRIM(COALESCE(CAST(json_extract(payload, '$.usageMissingReason') AS TEXT), '')) IN ('usage_missing_in_stream', 'upstream_response_failed')
                                OR TRIM(COALESCE(CAST(json_extract(payload, '$.streamTerminalEvent') AS TEXT), '')) != ''
                            )
                        )
                        OR (
                            response_raw_path IS NOT NULL
                            AND COALESCE(response_raw_size, LENGTH(raw_response)) >= 16384
                            AND json_valid(payload)
                            AND COALESCE(CAST(json_extract(payload, '$.endpoint') AS TEXT), '') = '/v1/responses'
                            AND COALESCE(json_extract(payload, '$.isStream'), 0) = 1
                            AND TRIM(COALESCE(failure_kind, '')) = ''
                        )
                    )
                )
              )
            ORDER BY id ASC
            LIMIT ?3
            "#;

const UPGRADE_FAILURE_CLASSIFICATION_QUERY: &str = "UPDATE codex_invocations SET status = ?1, error_message = ?2, failure_kind = ?3, failure_class = ?4, is_actionable = ?5, payload = ?6 WHERE id = ?7";
const UPDATE_FAILURE_CLASSIFICATION_QUERY: &str = "UPDATE codex_invocations SET failure_kind = ?1, failure_class = ?2, is_actionable = ?3 WHERE id = ?4";

struct FailureClassificationRowOutcome {
    updated: u64,
    updated_id: Option<i64>,
}

async fn process_failure_classification_row(
    tx: &mut SqliteConnection,
    row: &FailureClassificationBackfillRow,
    raw_path_fallback_root: Option<&Path>,
    samples: &mut Vec<String>,
) -> Result<FailureClassificationRowOutcome> {
    let existing_kind = row
        .failure_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let existing_class = row
        .failure_class
        .as_deref()
        .and_then(FailureClass::from_db_str);
    let existing_actionable = row.is_actionable.map(|value| value != 0);
    let response_failure =
        match parse_proxy_response_failure_from_persisted_record(row, raw_path_fallback_root) {
            Ok(result) => result,
            Err(err) => {
                push_backfill_sample(
                    samples,
                    format!(
                        "id={} reason=response_failure_parse_error err={err}",
                        row.id
                    ),
                );
                None
            }
        };

    if let Some(response_info) = response_failure
        .as_ref()
        .filter(|_| should_upgrade_to_upstream_response_failed(row, existing_kind.as_deref()))
    {
        let error_message = format_upstream_response_failed_message(response_info);
        let resolved = classify_invocation_failure(Some("http_200"), Some(&error_message));
        let next_payload = update_proxy_payload_failure_details(
            row.payload.as_deref(),
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
            response_info,
        );
        let affected = sqlx::query(UPGRADE_FAILURE_CLASSIFICATION_QUERY)
            .bind("http_200")
            .bind(&error_message)
            .bind(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
            .bind(resolved.failure_class.as_str())
            .bind(resolved.is_actionable as i64)
            .bind(next_payload)
            .bind(row.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        return Ok(FailureClassificationRowOutcome {
            updated: affected,
            updated_id: (affected > 0).then_some(row.id),
        });
    }

    let resolved = resolve_failure_classification(
        row.status.as_deref(),
        row.error_message.as_deref(),
        row.failure_kind.as_deref(),
        row.failure_class.as_deref(),
        row.is_actionable,
    );
    let next_kind = existing_kind.or(resolved.failure_kind.clone());
    if existing_class == Some(resolved.failure_class)
        && existing_actionable == Some(resolved.is_actionable)
        && row.failure_kind.as_deref().map(str::trim) == next_kind.as_deref()
    {
        return Ok(FailureClassificationRowOutcome {
            updated: 0,
            updated_id: None,
        });
    }
    let affected = sqlx::query(UPDATE_FAILURE_CLASSIFICATION_QUERY)
        .bind(next_kind.as_deref())
        .bind(resolved.failure_class.as_str())
        .bind(resolved.is_actionable as i64)
        .bind(row.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    Ok(FailureClassificationRowOutcome {
        updated: affected,
        updated_id: (affected > 0).then_some(row.id),
    })
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) async fn backfill_failure_classification(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<FailureClassificationBackfillSummary> {
    Ok(
        backfill_failure_classification_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

pub(crate) fn is_sqlite_lock_error(err: &anyhow::Error) -> bool {
    if err.chain().any(|cause| {
        let Some(sqlx_err) = cause.downcast_ref::<sqlx::Error>() else {
            return false;
        };
        let sqlx::Error::Database(db_err) = sqlx_err else {
            return false;
        };
        matches!(
            db_err.code().as_deref(),
            Some("5") | Some("6") | Some("SQLITE_BUSY") | Some("SQLITE_LOCKED")
        )
    }) {
        return true;
    }

    err.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("database is locked")
            || message.contains("database table is locked")
            || message.contains("sqlite_busy")
            || message.contains("sqlite_locked")
            || message.contains("(code: 5)")
            || message.contains("(code: 6)")
    })
}
