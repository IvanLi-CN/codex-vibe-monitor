#[cfg(test)]
async fn backfill_proxy_missing_costs(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let attempt_version = pricing_backfill_attempt_version(catalog);
    let requested_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
    let response_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
    let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(
        pool,
        &attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
    )
    .await?;
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        &attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_proxy_missing_costs_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    catalog: &PricingCatalog,
    attempt_version: &str,
) -> Result<ProxyCostBackfillSummary> {
    let requested_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
    let response_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
async fn run_cost_backfill_with_retry(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_missing_costs(pool, catalog).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_secs = BACKFILL_LOCK_RETRY_DELAY_SECS,
                    error = %err,
                    "proxy cost startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS)).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy cost startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

async fn backfill_proxy_prompt_cache_keys_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyPromptCacheKeyBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyPromptCacheKeyBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyPromptCacheKeyBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.promptCacheKey') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        let mut updates = Vec::new();
        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(prompt_cache_key) =
                extract_prompt_cache_key_from_request_body(&request_payload)
            else {
                summary.skipped_missing_key += 1;
                continue;
            };
            updates.push((candidate.id, prompt_cache_key));
        }

        if !updates.is_empty() {
            let mut tx = pool.begin().await?;
            let mut updated_ids = Vec::new();
            for (id, prompt_cache_key) in updates {
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET payload = json_remove(
                        json_set(
                            CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                            '$.promptCacheKey',
                            ?1
                        ),
                        '$.codexSessionId'
                    )
                    WHERE id = ?2
                      AND source = ?3
                      AND request_raw_path IS NOT NULL
                      AND (
                        payload IS NULL
                        OR NOT json_valid(payload)
                        OR json_extract(payload, '$.promptCacheKey') IS NULL
                        OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = ''
                      )
                    "#,
                )
                .bind(prompt_cache_key)
                .bind(id)
                .bind(SOURCE_PROXY)
                .execute(&mut *tx)
                .await?
                .rows_affected();
                summary.updated += affected;
                if affected > 0 {
                    updated_ids.push(id);
                }
            }
            if !updated_ids.is_empty() {
                recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
            }
            tx.commit().await?;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_prompt_cache_keys(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyPromptCacheKeyBackfillSummary> {
    Ok(
        backfill_proxy_prompt_cache_keys_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

async fn backfill_proxy_requested_service_tiers_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyRequestedServiceTierBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyRequestedServiceTierBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyRequestedServiceTierBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.requestedServiceTier') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(requested_service_tier) =
                extract_requested_service_tier_from_request_body(&request_payload)
            else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.requestedServiceTier',
                    ?1
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.requestedServiceTier') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(requested_service_tier)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_requested_service_tiers(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyRequestedServiceTierBackfillSummary> {
    Ok(backfill_proxy_requested_service_tiers_from_cursor(
        pool,
        0,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

async fn backfill_proxy_reasoning_efforts_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyReasoningEffortBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyReasoningEffortBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyReasoningEffortBackfillCandidate>(
            r#"
            SELECT id, request_raw_path
            FROM codex_invocations
            WHERE source = ?1
              AND request_raw_path IS NOT NULL
              AND id > ?2
              AND (
                payload IS NULL
                OR NOT json_valid(payload)
                OR json_extract(payload, '$.reasoningEffort') IS NULL
                OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = ''
              )
            ORDER BY id ASC
            LIMIT ?3
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(startup_backfill_query_limit(summary.scanned, scan_limit))
        .fetch_all(pool)
        .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;

            let raw_request =
                match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
                    Ok(content) => content,
                    Err(_) => {
                        summary.skipped_missing_file += 1;
                        push_backfill_sample(
                            &mut samples,
                            format!(
                                "id={} request_raw_path={} reason=missing_file",
                                candidate.id, candidate.request_raw_path
                            ),
                        );
                        continue;
                    }
                };

            let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
                Ok(payload) => payload,
                Err(_) => {
                    summary.skipped_invalid_json += 1;
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} request_raw_path={} reason=invalid_json",
                            candidate.id, candidate.request_raw_path
                        ),
                    );
                    continue;
                }
            };

            let Some(reasoning_effort) = extract_reasoning_effort_from_request_body(
                infer_proxy_capture_target_from_payload(&request_payload),
                &request_payload,
            ) else {
                summary.skipped_missing_effort += 1;
                continue;
            };

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = json_set(
                    CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                    '$.reasoningEffort',
                    ?1
                )
                WHERE id = ?2
                  AND source = ?3
                  AND request_raw_path IS NOT NULL
                  AND (
                    payload IS NULL
                    OR NOT json_valid(payload)
                    OR json_extract(payload, '$.reasoningEffort') IS NULL
                    OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = ''
                  )
                "#,
            )
            .bind(reasoning_effort)
            .bind(candidate.id)
            .bind(SOURCE_PROXY)
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_proxy_reasoning_efforts(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyReasoningEffortBackfillSummary> {
    Ok(
        backfill_proxy_reasoning_efforts_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

fn infer_proxy_capture_target_from_payload(value: &Value) -> ProxyCaptureTarget {
    if value.get("messages").is_some() || value.get("reasoning_effort").is_some() {
        ProxyCaptureTarget::ChatCompletions
    } else if value.get("previous_response_id").is_some() {
        ProxyCaptureTarget::ResponsesCompact
    } else {
        ProxyCaptureTarget::Responses
    }
}

#[derive(Debug, FromRow)]
struct InvocationServiceTierBackfillCandidate {
    id: i64,
    source: String,
    raw_response: String,
    response_raw_path: Option<String>,
    current_service_tier: Option<String>,
}

async fn backfill_invocation_service_tiers_from_cursor(
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
            r#"
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
            "#,
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
                        let (payload_for_parse, _) =
                            decode_response_payload_for_usage(&bytes, None);
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
                            &mut samples,
                            format!(
                                "id={} response_raw_path={} reason=missing_file",
                                candidate.id, path
                            ),
                        );
                        continue;
                    }
                }
            }

            let Some(service_tier) = service_tier else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            let should_mark_stream_backfill = candidate.source == SOURCE_PROXY;
            if candidate
                .current_service_tier
                .as_deref()
                .and_then(normalize_service_tier)
                .is_some_and(|current| current == service_tier)
                && !should_mark_stream_backfill
            {
                continue;
            }

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET payload = CASE
                    WHEN ?3 IS NULL THEN json_set(
                        CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                        '$.serviceTier',
                        ?1
                    )
                    ELSE json_set(
                        json_set(
                            CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                            '$.serviceTier',
                            ?1
                        ),
                        '$.serviceTierBackfillVersion',
                        ?3
                    )
                END
                WHERE id = ?2
                "#,
            )
            .bind(&service_tier)
            .bind(candidate.id)
            .bind(should_mark_stream_backfill.then_some(SERVICE_TIER_STREAM_BACKFILL_VERSION))
            .execute(pool)
            .await?
            .rows_affected();
            summary.updated += affected;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

#[cfg(test)]
async fn backfill_invocation_service_tiers(
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
struct FailureClassificationBackfillRow {
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

fn parse_proxy_response_capture_from_stored_bytes(
    target: ProxyCaptureTarget,
    bytes: &[u8],
    is_stream: bool,
) -> ResponseCaptureInfo {
    let (payload_for_parse, _) = decode_response_payload_for_usage(bytes, None);
    parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None)
}

fn format_upstream_response_failed_message(response_info: &ResponseCaptureInfo) -> String {
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

fn update_proxy_payload_failure_details(
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

fn should_upgrade_to_upstream_response_failed(
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

    invocation_status_is_success_like(row.status.as_deref(), row.error_message.as_deref())
        || existing_kind.is_none()
        || existing_kind == Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
}

fn parse_proxy_response_failure_from_persisted_record(
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

async fn backfill_failure_classification_from_cursor(
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
            r#"
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
                    LOWER(TRIM(COALESCE(status, ''))) != 'success'
                    AND TRIM(COALESCE(status, '')) != ''
                    AND TRIM(COALESCE(failure_kind, '')) = ''
                )
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) != 'success'
                    AND TRIM(COALESCE(failure_class, '')) = 'none'
                )
                OR (
                    source = ?2
                    AND LOWER(TRIM(COALESCE(status, ''))) = 'success'
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
            "#,
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

            let response_failure = match parse_proxy_response_failure_from_persisted_record(
                &row,
                raw_path_fallback_root,
            ) {
                Ok(result) => result,
                Err(err) => {
                    push_backfill_sample(
                        &mut samples,
                        format!(
                            "id={} reason=response_failure_parse_error err={err}",
                            row.id
                        ),
                    );
                    None
                }
            };

            if let Some(response_info) = response_failure.as_ref().filter(|_| {
                should_upgrade_to_upstream_response_failed(&row, existing_kind.as_deref())
            }) {
                let error_message = format_upstream_response_failed_message(response_info);
                let resolved = classify_invocation_failure(Some("http_200"), Some(&error_message));
                let next_payload = update_proxy_payload_failure_details(
                    row.payload.as_deref(),
                    Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED),
                    response_info,
                );
                let affected = sqlx::query(
                    r#"
                    UPDATE codex_invocations
                    SET status = ?1,
                        error_message = ?2,
                        failure_kind = ?3,
                        failure_class = ?4,
                        is_actionable = ?5,
                        payload = ?6
                    WHERE id = ?7
                    "#,
                )
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
                summary.updated += affected;
                if affected > 0 {
                    updated_ids.push(row.id);
                }
                continue;
            }

            let resolved = resolve_failure_classification(
                row.status.as_deref(),
                row.error_message.as_deref(),
                row.failure_kind.as_deref(),
                row.failure_class.as_deref(),
                row.is_actionable,
            );

            let next_kind = existing_kind.clone().or(resolved.failure_kind.clone());
            let should_update = existing_class != Some(resolved.failure_class)
                || existing_actionable != Some(resolved.is_actionable)
                || existing_kind != next_kind;

            if !should_update {
                continue;
            }

            let affected = sqlx::query(
                r#"
                UPDATE codex_invocations
                SET failure_kind = ?1,
                    failure_class = ?2,
                    is_actionable = ?3
                WHERE id = ?4
                "#,
            )
            .bind(next_kind.as_deref())
            .bind(resolved.failure_class.as_str())
            .bind(resolved.is_actionable as i64)
            .bind(row.id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            summary.updated += affected;
            if affected > 0 {
                updated_ids.push(row.id);
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

#[cfg(test)]
#[allow(dead_code)]
async fn backfill_failure_classification(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<FailureClassificationBackfillSummary> {
    Ok(
        backfill_failure_classification_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

fn is_sqlite_lock_error(err: &anyhow::Error) -> bool {
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

fn parse_proxy_capture_summary(payload: Option<&str>) -> (ProxyCaptureTarget, bool) {
    let mut target = ProxyCaptureTarget::Responses;
    let mut is_stream = false;

    let Some(raw) = payload else {
        return (target, is_stream);
    };
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return (target, is_stream);
    };

    if let Some(endpoint) = value.get("endpoint").and_then(|v| v.as_str()) {
        target = ProxyCaptureTarget::from_endpoint(endpoint);
    }
    if let Some(stream) = value.get("isStream").and_then(|v| v.as_bool()) {
        is_stream = stream;
    }

    (target, is_stream)
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn percentile_sorted_f64(sorted_values: &[f64], p: f64) -> f64 {
    if sorted_values.is_empty() {
        return 0.0;
    }
    if sorted_values.len() == 1 {
        return sorted_values[0];
    }
    let clamped = p.clamp(0.0, 1.0);
    let rank = clamped * (sorted_values.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        return sorted_values[lower];
    }
    let weight = rank - lower as f64;
    sorted_values[lower] + (sorted_values[upper] - sorted_values[lower]) * weight
}

fn next_proxy_request_id() -> u64 {
    NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
struct PoolRoutingReservation {
    account_id: i64,
    proxy_key: Option<String>,
    #[allow(dead_code)]
    created_at: Instant,
}

#[derive(Debug, Default, Clone)]
struct PoolRoutingReservationSnapshot {
    counts_by_account: HashMap<i64, i64>,
    proxy_keys_by_account: HashMap<i64, HashSet<String>>,
    reserved_proxy_keys: HashSet<String>,
}

impl PoolRoutingReservationSnapshot {
    fn count_for_account(&self, account_id: i64) -> i64 {
        self.counts_by_account
            .get(&account_id)
            .copied()
            .unwrap_or_default()
    }

    fn pinned_proxy_keys_for_account(
        &self,
        account_id: i64,
        valid_proxy_keys: &[String],
        occupied_proxy_keys: &HashSet<String>,
    ) -> Vec<String> {
        let Some(proxy_keys) = self.proxy_keys_by_account.get(&account_id) else {
            return Vec::new();
        };
        valid_proxy_keys
            .iter()
            .filter(|proxy_key| {
                proxy_keys.contains(proxy_key.as_str())
                    && !occupied_proxy_keys.contains(proxy_key.as_str())
            })
            .cloned()
            .collect()
    }

    fn reserved_proxy_keys_for_group(&self, valid_proxy_keys: &[String]) -> HashSet<String> {
        let valid_proxy_keys = valid_proxy_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        self.reserved_proxy_keys
            .iter()
            .filter(|proxy_key| valid_proxy_keys.contains(proxy_key.as_str()))
            .cloned()
            .collect()
    }
}

#[derive(Debug)]
struct PoolRoutingReservationDropGuard {
    state: Arc<AppState>,
    reservation_key: String,
    active: bool,
}

impl PoolRoutingReservationDropGuard {
    fn new(state: Arc<AppState>, reservation_key: String) -> Self {
        Self {
            state,
            reservation_key,
            active: true,
        }
    }

    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for PoolRoutingReservationDropGuard {
    fn drop(&mut self) {
        if self.active {
            release_pool_routing_reservation(self.state.as_ref(), &self.reservation_key);
        }
    }
}

fn build_pool_routing_reservation_key(proxy_request_id: u64) -> String {
    format!("pool-route-{proxy_request_id}")
}

fn pool_routing_reservation_count(state: &AppState, account_id: i64) -> i64 {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations
        .values()
        .filter(|reservation| reservation.account_id == account_id)
        .count() as i64
}

fn pool_routing_reservation_snapshot(state: &AppState) -> PoolRoutingReservationSnapshot {
    let reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    let mut snapshot = PoolRoutingReservationSnapshot::default();
    for reservation in reservations.values() {
        *snapshot
            .counts_by_account
            .entry(reservation.account_id)
            .or_default() += 1;
        if let Some(proxy_key) = reservation.proxy_key.as_deref() {
            snapshot.reserved_proxy_keys.insert(proxy_key.to_string());
            snapshot
                .proxy_keys_by_account
                .entry(reservation.account_id)
                .or_default()
                .insert(proxy_key.to_string());
        }
    }
    snapshot
}

fn reserve_pool_routing_account(
    state: &AppState,
    reservation_key: &str,
    account: &PoolResolvedAccount,
) {
    let proxy_key = match &account.forward_proxy_scope {
        ForwardProxyRouteScope::PinnedProxyKey(proxy_key) => Some(proxy_key.clone()),
        _ => None,
    };
    if account.routing_source == PoolRoutingSelectionSource::StickyReuse && proxy_key.is_none() {
        return;
    }
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations.insert(
        reservation_key.to_string(),
        PoolRoutingReservation {
            account_id: account.account_id,
            proxy_key,
            created_at: Instant::now(),
        },
    );
}

fn release_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    let mut reservations = state
        .pool_routing_reservations
        .lock()
        .expect("pool routing reservations mutex poisoned");
    reservations.remove(reservation_key);
}

fn consume_pool_routing_reservation(state: &AppState, reservation_key: &str) {
    release_pool_routing_reservation(state, reservation_key);
}

fn is_body_too_large_error(err: &reqwest::Error) -> bool {
    error_chain_contains(err, "length limit exceeded")
        || error_chain_contains(err, PROXY_REQUEST_BODY_LIMIT_EXCEEDED)
}

fn error_chain_contains(err: &(dyn std::error::Error + 'static), needle: &str) -> bool {
    if err.to_string().contains(needle) {
        return true;
    }
    let mut source = err.source();
    while let Some(inner) = source {
        if inner.to_string().contains(needle) {
            return true;
        }
        source = inner.source();
    }
    false
}

fn build_proxy_upstream_url(base: &Url, original_uri: &Uri) -> Result<Url> {
    if path_has_forbidden_dot_segment(original_uri.path()) {
        bail!(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED);
    }
    if has_invalid_percent_encoding(original_uri.path())
        || original_uri
            .query()
            .is_some_and(has_invalid_percent_encoding)
    {
        bail!(PROXY_INVALID_REQUEST_TARGET);
    }

    let host = base
        .host_str()
        .ok_or_else(|| anyhow!("OPENAI_UPSTREAM_BASE_URL is missing host"))?;
    let mut target = String::new();
    target.push_str(base.scheme());
    target.push_str("://");
    if !base.username().is_empty() {
        target.push_str(base.username());
        if let Some(password) = base.password() {
            target.push(':');
            target.push_str(password);
        }
        target.push('@');
    }
    if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        target.push('[');
        target.push_str(host);
        target.push(']');
    } else {
        target.push_str(host);
    }
    if let Some(port) = base.port() {
        target.push(':');
        target.push_str(&port.to_string());
    }

    let base_path = if base.path() == "/" {
        ""
    } else {
        base.path().trim_end_matches('/')
    };
    target.push_str(base_path);
    let request_path = original_uri.path();
    if !request_path.starts_with('/') {
        target.push('/');
    }
    target.push_str(request_path);
    if let Some(query) = original_uri.query() {
        target.push('?');
        target.push_str(query);
    }

    Url::parse(&target).context("failed to parse proxy upstream url")
}

fn path_has_forbidden_dot_segment(path: &str) -> bool {
    let mut candidate = path.to_string();
    for _ in 0..3 {
        if decoded_path_has_forbidden_dot_segment(&candidate) {
            return true;
        }
        let decoded = percent_decode_once_lossy(&candidate);
        if decoded == candidate {
            break;
        }
        candidate = decoded;
    }
    decoded_path_has_forbidden_dot_segment(&candidate)
}

fn has_invalid_percent_encoding(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%' {
            if idx + 2 >= bytes.len()
                || decode_hex_nibble(bytes[idx + 1]).is_none()
                || decode_hex_nibble(bytes[idx + 2]).is_none()
            {
                return true;
            }
            idx += 3;
            continue;
        }
        idx += 1;
    }
    false
}

fn decoded_path_has_forbidden_dot_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(is_forbidden_dot_segment)
}

fn is_forbidden_dot_segment(segment: &str) -> bool {
    segment == "." || segment == ".."
}

fn percent_decode_once_lossy(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'%'
            && idx + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                decode_hex_nibble(bytes[idx + 1]),
                decode_hex_nibble(bytes[idx + 2]),
            )
        {
            decoded.push((hi << 4) | lo);
            idx += 3;
            continue;
        }
        decoded.push(bytes[idx]);
        idx += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn connection_scoped_header_names(headers: &HeaderMap) -> HashSet<HeaderName> {
    let mut names = HashSet::new();
    for value in headers.get_all(header::CONNECTION).iter() {
        let Ok(raw) = value.to_str() else {
            continue;
        };
        for token in raw.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if let Ok(header_name) = HeaderName::from_bytes(token.as_bytes()) {
                names.insert(header_name);
            }
        }
    }
    names
}

pub(crate) fn should_forward_proxy_header(
    name: &HeaderName,
    connection_scoped: &HashSet<HeaderName>,
) -> bool {
    should_transport_proxy_header(name) && !connection_scoped.contains(name)
}

fn request_may_have_body(method: &Method, headers: &HeaderMap) -> bool {
    if headers.contains_key(header::TRANSFER_ENCODING) {
        return true;
    }
    if let Some(content_length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return content_length > 0;
    }
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn location_rewrite_upstream_base<'a>(
    pool_account: Option<&'a PoolResolvedAccount>,
    global_upstream_base_url: &'a Url,
) -> &'a Url {
    pool_account
        .map(|account| &account.upstream_base_url)
        .unwrap_or(global_upstream_base_url)
}

fn normalize_proxy_location_header(
    status: StatusCode,
    headers: &HeaderMap,
    upstream_base: &Url,
) -> Result<Option<String>> {
    let Some(raw_location) = headers.get(header::LOCATION) else {
        return Ok(None);
    };

    let raw_location = raw_location
        .to_str()
        .context("upstream Location header is not valid UTF-8")?;
    if raw_location.is_empty() {
        return Ok(None);
    }

    if !status.is_redirection() {
        return Ok(Some(raw_location.to_string()));
    }

    if raw_location.starts_with("//") {
        bail!("cross-origin redirect is not allowed");
    }

    if let Ok(parsed) = Url::parse(raw_location) {
        if !is_same_origin(&parsed, upstream_base) {
            bail!("cross-origin redirect is not allowed");
        }
        let mut normalized = rewrite_proxy_location_path(parsed.path(), upstream_base).to_string();
        if let Some(query) = parsed.query() {
            normalized.push('?');
            normalized.push_str(query);
        }
        if let Some(fragment) = parsed.fragment() {
            normalized.push('#');
            normalized.push_str(fragment);
        }
        return Ok(Some(normalized));
    }

    if raw_location.starts_with('/') {
        return Ok(Some(rewrite_proxy_relative_location(
            raw_location,
            upstream_base,
        )));
    }

    Ok(Some(raw_location.to_string()))
}

fn rewrite_proxy_relative_location(location: &str, upstream_base: &Url) -> String {
    let (path_and_query, fragment) = match location.split_once('#') {
        Some((pq, frag)) => (pq, Some(frag)),
        None => (location, None),
    };
    let (path, query) = match path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (path_and_query, None),
    };

    let mut rewritten = rewrite_proxy_location_path(path, upstream_base);
    if let Some(query) = query {
        rewritten.push('?');
        rewritten.push_str(query);
    }
    if let Some(fragment) = fragment {
        rewritten.push('#');
        rewritten.push_str(fragment);
    }
    rewritten
}

fn rewrite_proxy_location_path(upstream_path: &str, upstream_base: &Url) -> String {
    let base_path = upstream_base.path().trim_end_matches('/');
    if base_path.is_empty() || base_path == "/" {
        return upstream_path.to_string();
    }
    if upstream_path == base_path {
        return "/".to_string();
    }
    if let Some(stripped) = upstream_path.strip_prefix(base_path)
        && stripped.starts_with('/')
    {
        return stripped.to_string();
    }
    upstream_path.to_string()
}

fn is_same_origin(lhs: &Url, rhs: &Url) -> bool {
    lhs.scheme() == rhs.scheme()
        && lhs.host_str() == rhs.host_str()
        && effective_port(lhs) == effective_port(rhs)
}

fn effective_port(url: &Url) -> Option<u16> {
    url.port_or_known_default()
}

fn should_transport_proxy_header(name: &HeaderName) -> bool {
    !matches!(
        name.as_str(),
        "host"
            | "connection"
            | "proxy-connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "forwarded"
            | "via"
            | "x-real-ip"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
            | "x-forwarded-port"
            | "x-forwarded-client-cert"
    )
}

fn build_cors_layer(config: &AppConfig) -> CorsLayer {
    let allowed = config
        .cors_allowed_origins
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    let allow_origin = AllowOrigin::predicate(move |origin, _request| {
        let Ok(origin_raw) = origin.to_str() else {
            return false;
        };
        origin_allowed(origin_raw, &allowed)
    });
    CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods(Any)
        .allow_headers(Any)
}

fn origin_allowed(origin_raw: &str, configured: &HashSet<String>) -> bool {
    let Some(origin) = normalize_cors_origin(origin_raw) else {
        return false;
    };
    if configured.contains(&origin) {
        return true;
    }
    is_loopback_origin(origin_raw)
}

fn is_loopback_origin(origin_raw: &str) -> bool {
    let Ok(origin) = Url::parse(origin_raw) else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    origin
        .host_str()
        .map(is_loopback_authority_host)
        .unwrap_or(false)
}

fn parse_cors_allowed_origins_env(name: &str) -> Result<Vec<String>> {
    match env::var(name) {
        Ok(raw) => parse_cors_allowed_origins(&raw),
        Err(env::VarError::NotPresent) => Ok(Vec::new()),
        Err(err) => Err(anyhow!("failed to read {name}: {err}")),
    }
}

fn parse_cors_allowed_origins(raw: &str) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for candidate in raw.split(',').map(str::trim).filter(|v| !v.is_empty()) {
        let normalized = normalize_cors_origin(candidate)
            .ok_or_else(|| anyhow!("invalid {ENV_CORS_ALLOWED_ORIGINS} entry: {candidate}"))?;
        if seen.insert(normalized.clone()) {
            entries.push(normalized);
        }
    }
    Ok(entries)
}

fn normalize_cors_origin(origin_raw: &str) -> Option<String> {
    let origin = Url::parse(origin_raw).ok()?;
    if !matches!(origin.scheme(), "http" | "https") {
        return None;
    }
    if origin.cannot_be_a_base()
        || !origin.username().is_empty()
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
    {
        return None;
    }
    if origin.path() != "/" {
        return None;
    }

    let host = origin.host_str()?;
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_ascii_lowercase()
    };
    let scheme = origin.scheme().to_ascii_lowercase();
    let port = origin.port();
    let default_port = default_port_for_scheme(&scheme);

    if port.is_none() || port == default_port {
        Some(format!("{scheme}://{host}"))
    } else {
        Some(format!("{scheme}://{host}:{}", port?))
    }
}

fn is_models_list_path(path: &str) -> bool {
    path == "/v1/models"
}

// Browser-side CSRF mitigation for settings writes.
//
// This is intentionally not a full authentication mechanism: non-browser clients
// (CLI/automation) may omit Origin and are allowed by policy. The security boundary
// is deployment-level network isolation (trusted gateway only), documented in
// docs/deployment.md.
fn is_same_origin_settings_write(headers: &HeaderMap) -> bool {
    if matches!(
        header_value_as_str(headers, "sec-fetch-site"),
        Some(site)
            if site.eq_ignore_ascii_case("cross-site")
    ) {
        return false;
    }

    let Some(origin_raw) = headers.get(header::ORIGIN) else {
        // Non-browser clients may omit Origin (for example curl or internal tooling).
        // We only treat explicit browser cross-site signals as forbidden above.
        return true;
    };
    let Ok(origin) = origin_raw.to_str() else {
        return false;
    };
    let Ok(origin_url) = Url::parse(origin) else {
        return false;
    };
    if !matches!(origin_url.scheme(), "http" | "https") {
        return false;
    }

    let Some(origin_host) = origin_url.host_str() else {
        return false;
    };
    let Some((request_host, request_port)) =
        forwarded_or_host_authority(headers, origin_url.scheme())
    else {
        return false;
    };

    let origin_port = origin_url.port_or_known_default();
    if origin_host.eq_ignore_ascii_case(&request_host) && origin_port == request_port {
        return true;
    }

    // Dev loopback proxies (for example Vite on 60080 -> backend on 8080) may rewrite Host and/or port,
    // but both ends remain loopback. Allow that local-only mismatch.
    //
    // For non-loopback deployments behind reverse proxies, we accept trusted forwarded
    // host/proto/port headers for origin matching, but these headers are never relayed
    // to upstream/downstream proxy traffic (see should_proxy_header).
    is_loopback_authority_host(origin_host) && is_loopback_authority_host(&request_host)
}

fn forwarded_or_host_authority(
    headers: &HeaderMap,
    origin_scheme: &str,
) -> Option<(String, Option<u16>)> {
    if let Some(forwarded_host_raw) = header_value_as_str(headers, "x-forwarded-host") {
        // This service expects a single trusted edge gateway. If forwarded headers
        // arrive as a chain, treat it as unsupported/misconfigured and reject writes.
        let forwarded_host = single_forwarded_header_value(forwarded_host_raw)?;
        let authority = Authority::from_str(forwarded_host).ok()?;
        let forwarded_proto = match header_value_as_str(headers, "x-forwarded-proto") {
            Some(raw) => {
                let proto = single_forwarded_header_value(raw)?.to_ascii_lowercase();
                if proto == "http" || proto == "https" {
                    Some(proto)
                } else {
                    return None;
                }
            }
            None => None,
        };
        let scheme = forwarded_proto.as_deref().unwrap_or(origin_scheme);
        let forwarded_port = match header_value_as_str(headers, "x-forwarded-port") {
            Some(raw) => {
                let value = single_forwarded_header_value(raw)?;
                Some(value.parse::<u16>().ok()?)
            }
            None => None,
        };
        let port = authority
            .port_u16()
            .or(forwarded_port)
            .or_else(|| default_port_for_scheme(scheme));
        return Some((authority.host().to_string(), port));
    }

    let host_raw = headers.get(header::HOST)?;
    let host_value = host_raw.to_str().ok()?;
    let authority = Authority::from_str(host_value).ok()?;
    Some((
        authority.host().to_string(),
        authority
            .port_u16()
            .or_else(|| default_port_for_scheme(origin_scheme)),
    ))
}

fn single_forwarded_header_value(raw: &str) -> Option<&str> {
    let mut parts = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let first = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some(first)
}

fn default_port_for_scheme(scheme: &str) -> Option<u16> {
    match scheme {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn header_value_as_str<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    headers
        .get(HeaderName::from_static(name))
        .and_then(|value| value.to_str().ok())
}

fn extract_requester_ip(headers: &HeaderMap, peer_ip: Option<IpAddr>) -> Option<String> {
    if let Some(x_forwarded_for) = header_value_as_str(headers, "x-forwarded-for")
        && let Some(ip) = extract_first_ip_from_x_forwarded_for(x_forwarded_for)
    {
        return Some(ip);
    }

    if let Some(x_real_ip) = header_value_as_str(headers, "x-real-ip")
        && let Some(ip) = extract_ip_from_header_value(x_real_ip)
    {
        return Some(ip);
    }

    if let Some(forwarded) = header_value_as_str(headers, "forwarded")
        && let Some(ip) = extract_ip_from_forwarded_header(forwarded)
    {
        return Some(ip);
    }

    peer_ip.map(|ip| ip.to_string())
}

fn extract_sticky_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-sticky-key",
        "sticky-key",
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn extract_prompt_cache_key_from_headers(headers: &HeaderMap) -> Option<String> {
    for header_name in [
        "x-prompt-cache-key",
        "prompt-cache-key",
        "x-openai-prompt-cache-key",
    ] {
        if let Some(raw_value) = header_value_as_str(headers, header_name) {
            let candidate = raw_value
                .split(',')
                .next()
                .map(str::trim)
                .unwrap_or(raw_value.trim())
                .trim_matches('"');
            if !candidate.is_empty() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

fn extract_first_ip_from_x_forwarded_for(raw: &str) -> Option<String> {
    let first = raw.split(',').next()?.trim();
    extract_ip_from_header_value(first)
}

fn extract_ip_from_forwarded_header(raw: &str) -> Option<String> {
    for entry in raw.split(',') {
        for segment in entry.split(';') {
            let pair = segment.trim();
            if pair.len() >= 4 && pair[..4].eq_ignore_ascii_case("for=") {
                let value = &pair[4..];
                if let Some(ip) = extract_ip_from_header_value(value) {
                    return Some(ip);
                }
            }
        }
    }
    None
}

fn extract_ip_from_header_value(raw: &str) -> Option<String> {
    let normalized = raw.trim().trim_matches('"');
    if normalized.is_empty()
        || normalized.eq_ignore_ascii_case("unknown")
        || normalized.starts_with('_')
    {
        return None;
    }

    if let Some(value) = normalized.strip_prefix("for=") {
        return extract_ip_from_header_value(value);
    }

    if normalized.starts_with('[')
        && let Some(end) = normalized.find(']')
        && let Ok(ip) = normalized[1..end].parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    if let Ok(ip) = normalized.parse::<IpAddr>() {
        return Some(ip.to_string());
    }

    if let Some((host, port)) = normalized.rsplit_once(':')
        && !host.contains(':')
        && port.parse::<u16>().is_ok()
        && let Ok(ip) = host.parse::<IpAddr>()
    {
        return Some(ip.to_string());
    }

    None
}

fn is_loopback_authority_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn build_preset_models_payload(enabled_model_ids: &[String]) -> Value {
    let data = enabled_model_ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "owned_by": "proxy",
                "created": 0
            })
        })
        .collect::<Vec<_>>();
    json!({
        "object": "list",
        "data": data
    })
}

fn merge_models_payload_with_upstream(
    upstream_payload: &Value,
    enabled_model_ids: &[String],
) -> Result<Value> {
    let upstream_items = upstream_payload
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("upstream models payload missing data array"))?;
    let mut merged = build_preset_models_payload(enabled_model_ids)
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut seen_ids: HashSet<String> = enabled_model_ids.iter().cloned().collect();

    for item in upstream_items {
        if let Some(id) = item.get("id").and_then(|v| v.as_str())
            && seen_ids.insert(id.to_string())
        {
            merged.push(item.clone());
        }
    }

    Ok(json!({
        "object": "list",
        "data": merged
    }))
}
