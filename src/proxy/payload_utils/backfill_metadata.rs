use super::*;

#[cfg(test)]
pub(crate) async fn backfill_proxy_missing_costs(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<ProxyCostBackfillSummary> {
    let attempt_version = pricing_backfill_attempt_version(catalog);
    let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(pool).await?;
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        &attempt_version,
        None,
        None,
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
    Ok(backfill_proxy_missing_costs_from_cursor(
        pool,
        0,
        snapshot_max_id,
        catalog,
        attempt_version,
        None,
        None,
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
    let mut committed_cursor_id = start_after_id;
    let mut committed_scanned = 0_u64;
    let mut committed_updated = 0_u64;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    let result = async {
        loop {
            if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed)
            {
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
                    match read_proxy_raw_bytes(&candidate.request_raw_path, raw_path_fallback_root)
                    {
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
                let mut updated_prompt_cache_keys = HashSet::new();
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
                    .bind(&prompt_cache_key)
                    .bind(id)
                    .bind(SOURCE_PROXY)
                    .execute(&mut *tx)
                    .await?
                    .rows_affected();
                    summary.updated += affected;
                    if affected > 0 {
                        updated_ids.push(id);
                        updated_prompt_cache_keys.insert(prompt_cache_key);
                    }
                }
                if !updated_ids.is_empty() {
                    recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids)
                        .await?;
                }
                tx.commit().await?;
                committed_cursor_id = last_seen_id;
                committed_scanned = summary.scanned;
                committed_updated = summary.updated;
                if !updated_prompt_cache_keys.is_empty() {
                    // The trigger keeps the aggregate refresh durable; historical aggregation belongs
                    // to the pressure-gated materialization task, not this metadata backfill pass.
                    for prompt_cache_key in &updated_prompt_cache_keys {
                        crate::ensure_prompt_cache_conversation_row(pool, prompt_cache_key).await?;
                    }
                }
            }
            committed_cursor_id = last_seen_id;
            committed_scanned = summary.scanned;
            committed_updated = summary.updated;
        }

        Ok(BackfillBatchOutcome {
            summary,
            next_cursor_id: committed_cursor_id,
            hit_budget,
            samples: samples.clone(),
        })
    }
    .await;
    result.map_err(|error| {
        anyhow::Error::new(crate::BackfillPartialFailure {
            source: error,
            next_cursor_id: committed_cursor_id,
            scanned: committed_scanned,
            updated: committed_updated,
        })
    })
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
    let mut committed_cursor_id = start_after_id;
    let mut committed_scanned = 0_u64;
    let mut committed_updated = 0_u64;
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
        .await
        .map_err(|error| {
            anyhow::Error::new(crate::BackfillPartialFailure {
                source: error.into(),
                next_cursor_id: committed_cursor_id,
                scanned: committed_scanned,
                updated: committed_updated,
            })
        })?;

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
                        committed_cursor_id = candidate.id;
                        committed_scanned = summary.scanned;
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
                    committed_cursor_id = candidate.id;
                    committed_scanned = summary.scanned;
                    continue;
                }
            };

            let Some(requested_service_tier) =
                extract_requested_service_tier_from_request_body(&request_payload)
            else {
                summary.skipped_missing_tier += 1;
                committed_cursor_id = candidate.id;
                committed_scanned = summary.scanned;
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
            .await
            .map_err(|error| {
                anyhow::Error::new(crate::BackfillPartialFailure {
                    source: error.into(),
                    next_cursor_id: committed_cursor_id,
                    scanned: committed_scanned,
                    updated: committed_updated,
                })
            })?
            .rows_affected();
            summary.updated += affected;
            committed_updated = summary.updated;
            committed_cursor_id = candidate.id;
            committed_scanned = summary.scanned;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: committed_cursor_id,
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
    let mut committed_cursor_id = start_after_id;
    let mut committed_scanned = 0_u64;
    let mut committed_updated = 0_u64;
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
        .await
        .map_err(|error| {
            anyhow::Error::new(crate::BackfillPartialFailure {
                source: error.into(),
                next_cursor_id: committed_cursor_id,
                scanned: committed_scanned,
                updated: committed_updated,
            })
        })?;

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
                        committed_cursor_id = candidate.id;
                        committed_scanned = summary.scanned;
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
                    committed_cursor_id = candidate.id;
                    committed_scanned = summary.scanned;
                    continue;
                }
            };

            let Some(reasoning_effort) = extract_reasoning_effort_from_request_body(
                infer_proxy_capture_target_from_payload(&request_payload),
                &request_payload,
            ) else {
                summary.skipped_missing_effort += 1;
                committed_cursor_id = candidate.id;
                committed_scanned = summary.scanned;
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
            .await
            .map_err(|error| {
                anyhow::Error::new(crate::BackfillPartialFailure {
                    source: error.into(),
                    next_cursor_id: committed_cursor_id,
                    scanned: committed_scanned,
                    updated: committed_updated,
                })
            })?
            .rows_affected();
            summary.updated += affected;
            committed_updated = summary.updated;
            committed_cursor_id = candidate.id;
            committed_scanned = summary.scanned;
        }
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: committed_cursor_id,
        hit_budget,
        samples,
    })
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
