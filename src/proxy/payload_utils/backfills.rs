use super::*;

#[cfg(test)]
pub(crate) async fn backfill_proxy_missing_costs(
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
        ProxyCostBackfillRequest {
            start_after_id: 0,
            snapshot_max_id,
            catalog,
            attempt_version: &attempt_version,
            requested_tier_price_version: &requested_tier_price_version,
            response_tier_price_version: &response_tier_price_version,
            scan_limit: None,
            max_elapsed: None,
        },
    )
    .await?
    .summary)
}

#[cfg(test)]
#[allow(dead_code)]
pub(crate) async fn backfill_proxy_missing_costs_up_to_id(
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
        ProxyCostBackfillRequest {
            start_after_id: 0,
            snapshot_max_id,
            catalog,
            attempt_version,
            requested_tier_price_version: &requested_tier_price_version,
            response_tier_price_version: &response_tier_price_version,
            scan_limit: None,
            max_elapsed: None,
        },
    )
    .await?
    .summary)
}

#[cfg(test)]
pub(crate) const TEST_PROXY_COST_BACKFILL_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

#[cfg(test)]
pub(crate) async fn run_cost_backfill_with_retry(
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
                    retry_delay_ms = TEST_PROXY_COST_BACKFILL_LOCK_RETRY_DELAY.as_millis() as u64,
                    error = %err,
                    "proxy cost startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(TEST_PROXY_COST_BACKFILL_LOCK_RETRY_DELAY).await;
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

pub(crate) async fn backfill_proxy_prompt_cache_keys_from_cursor(
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
            if let Some(update) = read_prompt_cache_key_update(
                &candidate,
                raw_path_fallback_root,
                &mut summary,
                &mut samples,
            ) {
                updates.push(update);
            }
        }

        apply_prompt_cache_key_updates(pool, updates, &mut summary).await?;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

fn read_prompt_cache_key_update(
    candidate: &ProxyPromptCacheKeyBackfillCandidate,
    raw_path_fallback_root: Option<&Path>,
    summary: &mut ProxyPromptCacheKeyBackfillSummary,
    samples: &mut Vec<String>,
) -> Option<(i64, String)> {
    let raw_request =
        match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root) {
            Ok(content) => content,
            Err(_) => {
                summary.skipped_missing_file += 1;
                push_backfill_sample(
                    samples,
                    format!(
                        "id={} request_raw_path={} reason=missing_file",
                        candidate.id, candidate.request_raw_path
                    ),
                );
                return None;
            }
        };
    let request_payload = match serde_json::from_slice::<Value>(&raw_request) {
        Ok(payload) => payload,
        Err(_) => {
            summary.skipped_invalid_json += 1;
            push_backfill_sample(
                samples,
                format!(
                    "id={} request_raw_path={} reason=invalid_json",
                    candidate.id, candidate.request_raw_path
                ),
            );
            return None;
        }
    };
    let Some(prompt_cache_key) = extract_prompt_cache_key_from_request_body(&request_payload)
    else {
        summary.skipped_missing_key += 1;
        return None;
    };
    Some((candidate.id, prompt_cache_key))
}

async fn apply_prompt_cache_key_updates(
    pool: &Pool<Sqlite>,
    updates: Vec<(i64, String)>,
    summary: &mut ProxyPromptCacheKeyBackfillSummary,
) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    let mut updated_ids = Vec::new();
    for (id, prompt_cache_key) in updates {
        let affected = sqlx::query(
            r#"UPDATE codex_invocations
               SET payload = json_remove(json_set(
                   CASE WHEN json_valid(payload) THEN payload ELSE '{}' END,
                   '$.promptCacheKey', ?1), '$.codexSessionId')
               WHERE id = ?2 AND source = ?3 AND request_raw_path IS NOT NULL
                 AND (payload IS NULL OR NOT json_valid(payload)
                   OR json_extract(payload, '$.promptCacheKey') IS NULL
                   OR TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) = '')"#,
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
    Ok(())
}

#[cfg(test)]
pub(crate) async fn backfill_proxy_prompt_cache_keys(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyPromptCacheKeyBackfillSummary> {
    Ok(
        backfill_proxy_prompt_cache_keys_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

pub(crate) async fn backfill_proxy_requested_service_tiers_from_cursor(
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

            let Some(request_payload) = read_backfill_request_payload(
                &candidate.request_raw_path,
                raw_path_fallback_root,
                candidate.id,
                &mut summary.skipped_missing_file,
                &mut summary.skipped_invalid_json,
                &mut samples,
            ) else {
                continue;
            };

            let Some(requested_service_tier) =
                extract_requested_service_tier_from_request_body(&request_payload)
            else {
                summary.skipped_missing_tier += 1;
                continue;
            };

            summary.updated +=
                update_requested_service_tier(pool, candidate.id, &requested_service_tier).await?;
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
pub(crate) async fn backfill_proxy_requested_service_tiers(
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

pub(crate) async fn backfill_proxy_reasoning_efforts_from_cursor(
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

            let Some(request_payload) = read_backfill_request_payload(
                &candidate.request_raw_path,
                raw_path_fallback_root,
                candidate.id,
                &mut summary.skipped_missing_file,
                &mut summary.skipped_invalid_json,
                &mut samples,
            ) else {
                continue;
            };

            let Some(reasoning_effort) = extract_reasoning_effort_from_request_body(
                infer_proxy_capture_target_from_payload(&request_payload),
                &request_payload,
            ) else {
                summary.skipped_missing_effort += 1;
                continue;
            };

            summary.updated +=
                update_reasoning_effort(pool, candidate.id, &reasoning_effort).await?;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

fn read_backfill_request_payload(
    path: &str,
    raw_path_fallback_root: Option<&Path>,
    id: i64,
    skipped_missing_file: &mut u64,
    skipped_invalid_json: &mut u64,
    samples: &mut Vec<String>,
) -> Option<Value> {
    let raw_request = match read_proxy_raw_bytes(path, raw_path_fallback_root) {
        Ok(content) => content,
        Err(_) => {
            *skipped_missing_file += 1;
            push_backfill_sample(
                samples,
                format!("id={id} request_raw_path={path} reason=missing_file"),
            );
            return None;
        }
    };
    match serde_json::from_slice::<Value>(&raw_request) {
        Ok(payload) => Some(payload),
        Err(_) => {
            *skipped_invalid_json += 1;
            push_backfill_sample(
                samples,
                format!("id={id} request_raw_path={path} reason=invalid_json"),
            );
            None
        }
    }
}

async fn update_requested_service_tier(
    pool: &Pool<Sqlite>,
    id: i64,
    service_tier: &str,
) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE codex_invocations SET payload = json_set(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END, '$.requestedServiceTier', ?1) WHERE id = ?2 AND source = ?3 AND request_raw_path IS NOT NULL AND (payload IS NULL OR NOT json_valid(payload) OR json_extract(payload, '$.requestedServiceTier') IS NULL OR TRIM(CAST(json_extract(payload, '$.requestedServiceTier') AS TEXT)) = '')",
    )
    .bind(service_tier)
    .bind(id)
    .bind(SOURCE_PROXY)
    .execute(pool)
    .await?
    .rows_affected())
}

async fn update_reasoning_effort(pool: &Pool<Sqlite>, id: i64, effort: &str) -> Result<u64> {
    Ok(sqlx::query(
        "UPDATE codex_invocations SET payload = json_set(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END, '$.reasoningEffort', ?1) WHERE id = ?2 AND source = ?3 AND request_raw_path IS NOT NULL AND (payload IS NULL OR NOT json_valid(payload) OR json_extract(payload, '$.reasoningEffort') IS NULL OR TRIM(CAST(json_extract(payload, '$.reasoningEffort') AS TEXT)) = '')",
    )
    .bind(effort)
    .bind(id)
    .bind(SOURCE_PROXY)
    .execute(pool)
    .await?
    .rows_affected())
}

#[cfg(test)]
pub(crate) async fn backfill_proxy_reasoning_efforts(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyReasoningEffortBackfillSummary> {
    Ok(
        backfill_proxy_reasoning_efforts_from_cursor(pool, 0, raw_path_fallback_root, None, None)
            .await?
            .summary,
    )
}

pub(crate) fn infer_proxy_capture_target_from_payload(value: &Value) -> ProxyCaptureTarget {
    if value.get("messages").is_some() || value.get("reasoning_effort").is_some() {
        ProxyCaptureTarget::ChatCompletions
    } else if value.get("previous_response_id").is_some() {
        ProxyCaptureTarget::ResponsesCompact
    } else {
        ProxyCaptureTarget::Responses
    }
}

pub(crate) fn infer_image_intent_from_request_body(
    target: ProxyCaptureTarget,
    value: &Value,
) -> ImageIntent {
    infer_image_intent_from_request_body_with_codex_namespace(target, value, true)
}

/// Determines whether an upstream-hosted image capability is required. The Codex
/// `image_gen.imagegen` namespace is executed by the Codex client, so advertising it
/// must remain observable as image intent without affecting account capability routing.
pub(crate) fn infer_hosted_image_intent_from_request_body(
    target: ProxyCaptureTarget,
    value: &Value,
) -> ImageIntent {
    infer_image_intent_from_request_body_with_codex_namespace(target, value, false)
}

fn infer_image_intent_from_request_body_with_codex_namespace(
    target: ProxyCaptureTarget,
    value: &Value,
    include_codex_namespace: bool,
) -> ImageIntent {
    let input_contains_hosted_image_tool = value.get("input").is_some_and(|input| match input {
        Value::Array(input) => input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("additional_tools")
                && openai_json_tools_contain_image_generation(item.get("tools"))
        }),
        Value::Object(input) => {
            openai_json_tools_contain_image_generation(input.get("additional_tools"))
        }
        _ => false,
    });
    let input_contains_codex_imagegen = value.get("input").is_some_and(|input| match input {
        Value::Array(input) => input.iter().any(|item| {
            item.get("type").and_then(Value::as_str) == Some("additional_tools")
                && openai_json_tools_contain_codex_imagegen(item.get("tools"))
        }),
        Value::Object(input) => {
            openai_json_tools_contain_codex_imagegen(input.get("additional_tools"))
        }
        _ => false,
    });
    match target {
        ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits => {
            ImageIntent::DirectImage
        }
        ProxyCaptureTarget::ChatCompletions | ProxyCaptureTarget::StandaloneSearch => {
            ImageIntent::Unknown
        }
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            if value
                .get("model")
                .and_then(|entry| entry.as_str())
                .is_some_and(is_openai_image_generation_model)
                || openai_json_tools_contain_image_generation(value.get("tools"))
                || openai_json_tool_choice_selects_image_generation(value.get("tool_choice"))
                || input_contains_hosted_image_tool
                || (include_codex_namespace
                    && (openai_json_tools_contain_codex_imagegen(value.get("tools"))
                        || input_contains_codex_imagegen))
            {
                ImageIntent::Yes
            } else {
                ImageIntent::No
            }
        }
    }
}

pub(crate) fn request_declares_remote_v2_compaction(value: &Value) -> bool {
    fn entry_declares_remote_v2_compaction(entry: &Value) -> bool {
        let Some(object) = entry.as_object() else {
            return false;
        };
        object
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "compaction")
            && object.contains_key("compact_threshold")
    }

    let Some(context_management) = value.get("context_management") else {
        return false;
    };
    if let Some(entries) = context_management.as_array() {
        return entries.iter().any(entry_declares_remote_v2_compaction);
    }
    entry_declares_remote_v2_compaction(context_management)
}

pub(crate) fn is_openai_image_generation_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("gpt-image-")
}

pub(crate) fn openai_json_tools_contain_image_generation(tools: Option<&Value>) -> bool {
    let Some(Value::Array(items)) = tools else {
        return false;
    };
    items.iter().any(|item| {
        item.get("type")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.trim() == "image_generation")
    })
}

pub(crate) fn openai_json_tools_contain_codex_imagegen(tools: Option<&Value>) -> bool {
    let Some(Value::Array(items)) = tools else {
        return false;
    };
    items.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("image_gen.imagegen")
            || (item.get("type").and_then(Value::as_str) == Some("namespace")
                && item.get("name").and_then(Value::as_str) == Some("image_gen")
                && item
                    .get("tools")
                    .and_then(Value::as_array)
                    .is_some_and(|namespace_tools| {
                        namespace_tools.iter().any(|tool| {
                            tool.get("type").and_then(Value::as_str) == Some("function")
                                && tool.get("name").and_then(Value::as_str) == Some("imagegen")
                        })
                    }))
    })
}

pub(crate) fn openai_json_tool_choice_selects_image_generation(choice: Option<&Value>) -> bool {
    let Some(choice) = choice else {
        return false;
    };
    if let Some(value) = choice.as_str() {
        return value.trim() == "image_generation";
    }
    if !choice.is_object() {
        return false;
    }
    choice
        .get("type")
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.trim() == "image_generation")
        || choice
            .get("tool")
            .and_then(|tool| tool.get("type"))
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.trim() == "image_generation")
        || choice
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.trim() == "image_generation")
}
