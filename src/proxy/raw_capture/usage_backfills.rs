use super::*;
pub(crate) async fn current_proxy_usage_backfill_snapshot_max_id(
    pool: &Pool<Sqlite>,
) -> Result<i64> {
    let shared_live_cursor =
        load_hourly_rollup_live_progress(pool, HOURLY_ROLLUP_DATASET_INVOCATIONS).await?;
    let max_live_id =
        sqlx::query_scalar::<_, i64>("SELECT COALESCE(MAX(id), 0) FROM codex_invocations")
            .fetch_one(pool)
            .await?;
    Ok(shared_live_cursor.max(max_live_id))
}

pub(crate) async fn backfill_proxy_usage_tokens_from_cursor(
    pool: &Pool<Sqlite>,
    start_after_id: i64,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
    scan_limit: Option<u64>,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyUsageBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyUsageBackfillSummary::default();
    let mut last_seen_id = start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(started_at, summary.scanned, scan_limit, max_elapsed) {
            hit_budget = true;
            break;
        }

        let candidates = sqlx::query_as::<_, ProxyUsageBackfillCandidate>(
            r#"
            SELECT id, response_raw_path, payload
            FROM codex_invocations
            WHERE source = ?1
              AND LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'warning_success')
              AND total_tokens IS NULL
              AND response_raw_path IS NOT NULL
              AND id > ?2
              AND id <= ?3
            ORDER BY id ASC
            LIMIT ?4
            "#,
        )
        .bind(SOURCE_PROXY)
        .bind(last_seen_id)
        .bind(snapshot_max_id)
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
            if let Some(update) = read_proxy_usage_update(
                &candidate,
                raw_path_fallback_root,
                &mut summary,
                &mut samples,
            ) {
                updates.push(update);
            }
        }

        summary.updated += apply_proxy_usage_updates(pool, updates).await?;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

fn read_proxy_usage_update(
    candidate: &ProxyUsageBackfillCandidate,
    raw_path_fallback_root: Option<&Path>,
    summary: &mut ProxyUsageBackfillSummary,
    samples: &mut Vec<String>,
) -> Option<ProxyUsageBackfillUpdate> {
    let raw_response =
        match read_proxy_raw_bytes(&candidate.response_raw_path, raw_path_fallback_root) {
            Ok(content) => content,
            Err(_) => {
                summary.skipped_missing_file += 1;
                push_backfill_sample(
                    samples,
                    format!(
                        "id={} response_raw_path={} reason=missing_file",
                        candidate.id, candidate.response_raw_path
                    ),
                );
                return None;
            }
        };
    let (target, is_stream) = parse_proxy_capture_summary(candidate.payload.as_deref());
    let (payload_for_parse, decode_error) = decode_response_payload_for_usage(&raw_response, None);
    let usage =
        parse_target_response_payload(target, payload_for_parse.as_ref(), is_stream, None).usage;
    let has_usage = usage.total_tokens.is_some()
        || usage.input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.cache_input_tokens.is_some()
        || usage.reasoning_tokens.is_some();
    if !has_usage {
        if decode_error.is_some() {
            summary.skipped_decode_error += 1;
        } else {
            summary.skipped_without_usage += 1;
        }
        return None;
    }
    Some(ProxyUsageBackfillUpdate {
        id: candidate.id,
        usage,
    })
}

async fn apply_proxy_usage_updates(
    pool: &Pool<Sqlite>,
    updates: Vec<ProxyUsageBackfillUpdate>,
) -> Result<u64> {
    if updates.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await?;
    let mut updated = 0_u64;
    let mut updated_ids = Vec::new();
    for update in updates {
        let affected = sqlx::query(
            "UPDATE codex_invocations SET input_tokens = ?1, output_tokens = ?2, cache_input_tokens = ?3, reasoning_tokens = ?4, total_tokens = ?5 WHERE id = ?6 AND source = ?7 AND total_tokens IS NULL",
        )
        .bind(update.usage.input_tokens)
        .bind(update.usage.output_tokens)
        .bind(update.usage.cache_input_tokens)
        .bind(update.usage.reasoning_tokens)
        .bind(update.usage.total_tokens)
        .bind(update.id)
        .bind(SOURCE_PROXY)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        updated += affected;
        if affected > 0 {
            updated_ids.push(update.id);
        }
    }
    if !updated_ids.is_empty() {
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
    }
    tx.commit().await?;
    Ok(updated)
}

#[cfg(test)]
pub(crate) async fn backfill_proxy_usage_tokens(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let snapshot_max_id = current_proxy_usage_backfill_snapshot_max_id(pool).await?;
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
pub(crate) async fn backfill_proxy_usage_tokens_up_to_id(
    pool: &Pool<Sqlite>,
    snapshot_max_id: i64,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    Ok(backfill_proxy_usage_tokens_from_cursor(
        pool,
        0,
        snapshot_max_id,
        raw_path_fallback_root,
        None,
        None,
    )
    .await?
    .summary)
}

#[cfg(test)]
pub(crate) const TEST_PROXY_USAGE_BACKFILL_LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

#[cfg(test)]
pub(crate) async fn run_backfill_with_retry(
    pool: &Pool<Sqlite>,
    raw_path_fallback_root: Option<&Path>,
) -> Result<ProxyUsageBackfillSummary> {
    let mut attempt = 1_u32;
    loop {
        match backfill_proxy_usage_tokens(pool, raw_path_fallback_root).await {
            Ok(summary) => return Ok(summary),
            Err(err)
                if attempt < BACKFILL_LOCK_RETRY_MAX_ATTEMPTS && is_sqlite_lock_error(&err) =>
            {
                warn!(
                    attempt,
                    max_attempts = BACKFILL_LOCK_RETRY_MAX_ATTEMPTS,
                    retry_delay_ms = TEST_PROXY_USAGE_BACKFILL_LOCK_RETRY_DELAY.as_millis() as u64,
                    error = %err,
                    "proxy usage startup backfill hit sqlite lock; retrying"
                );
                attempt += 1;
                sleep(TEST_PROXY_USAGE_BACKFILL_LOCK_RETRY_DELAY).await;
            }
            Err(err) => {
                return Err(err).with_context(|| {
                    format!(
                        "proxy usage startup backfill failed after {attempt}/{} attempt(s)",
                        BACKFILL_LOCK_RETRY_MAX_ATTEMPTS
                    )
                });
            }
        }
    }
}

pub(crate) async fn current_proxy_cost_backfill_snapshot_max_id(
    pool: &Pool<Sqlite>,
    attempt_version: &str,
    requested_tier_price_version: &str,
    response_tier_price_version: &str,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        r#"
        WITH base AS (
            SELECT
                inv.id,
                inv.cost,
                inv.price_version,
                CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requestedServiceTier') = 'text'
                    THEN json_extract(inv.payload, '$.requestedServiceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requested_service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.requested_service_tier') END AS requested_service_tier,
                CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billingServiceTier') = 'text'
                    THEN json_extract(inv.payload, '$.billingServiceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billing_service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.billing_service_tier') END AS billing_service_tier,
                CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.serviceTier') = 'text'
                    THEN json_extract(inv.payload, '$.serviceTier')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.service_tier') = 'text'
                    THEN json_extract(inv.payload, '$.service_tier') END AS service_tier,
                CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamAccountKind') = 'text'
                    THEN json_extract(inv.payload, '$.upstreamAccountKind')
                  WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_account_kind') = 'text'
                    THEN json_extract(inv.payload, '$.upstream_account_kind') END AS snapshot_upstream_account_kind,
                acc.kind AS live_upstream_account_kind,
                CASE WHEN acc.created_at IS NOT NULL AND TRIM(CAST(acc.created_at AS TEXT)) != ''
                    AND inv.occurred_at IS NOT NULL AND TRIM(CAST(inv.occurred_at AS TEXT)) != ''
                    AND julianday(acc.created_at) <= julianday(inv.occurred_at)
                    AND (acc.updated_at IS NULL OR TRIM(CAST(acc.updated_at AS TEXT)) = ''
                      OR julianday(acc.updated_at) <= julianday(inv.occurred_at)) THEN 1 ELSE 0 END AS live_upstream_account_snapshot_safe
            FROM codex_invocations inv
            LEFT JOIN pool_upstream_accounts acc
              ON acc.id = CASE
                  WHEN json_valid(inv.payload)
                    THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER)
                END
            WHERE inv.source = ?1
              AND LOWER(TRIM(COALESCE(inv.status, ''))) IN ('success', 'warning_success', 'failed')
              AND inv.model IS NOT NULL
              AND (
                  COALESCE(inv.input_tokens, 0) > 0
                  OR COALESCE(inv.output_tokens, 0) > 0
                  OR COALESCE(inv.cache_input_tokens, 0) > 0
                  OR COALESCE(inv.reasoning_tokens, 0) > 0
              )
        ),
        cost_candidates AS (
            SELECT
                *,
                CASE
                  WHEN LOWER(TRIM(COALESCE(
                        snapshot_upstream_account_kind,
                        CASE WHEN live_upstream_account_snapshot_safe = 1 THEN live_upstream_account_kind END,
                        ''
                    ))) = ?4
                    AND TRIM(COALESCE(requested_service_tier, '')) != ''
                  THEN 1
                  ELSE 0
                END AS uses_requested_tier_strategy
            FROM base
        )
        SELECT COALESCE(MAX(id), 0)
        FROM cost_candidates
        WHERE (
            uses_requested_tier_strategy = 1
            AND (
                LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(requested_service_tier, '')))
                OR (cost IS NULL AND (price_version IS NULL OR price_version != ?2))
                OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?3))
            )
        )
        OR (
            uses_requested_tier_strategy = 0
            AND (
                LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(service_tier, '')))
                OR (cost IS NULL AND (price_version IS NULL OR price_version != ?2))
                OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?5))
            )
        )
        "#,
    )
    .bind(SOURCE_PROXY)
    .bind(attempt_version)
    .bind(requested_tier_price_version)
    .bind(API_KEYS_BILLING_ACCOUNT_KIND)
    .bind(response_tier_price_version)
    .fetch_one(pool)
    .await?)
}

pub(crate) struct ProxyCostBackfillRequest<'a> {
    pub(crate) start_after_id: i64,
    pub(crate) snapshot_max_id: i64,
    pub(crate) catalog: &'a PricingCatalog,
    pub(crate) attempt_version: &'a str,
    pub(crate) requested_tier_price_version: &'a str,
    pub(crate) response_tier_price_version: &'a str,
    pub(crate) scan_limit: Option<u64>,
    pub(crate) max_elapsed: Option<Duration>,
}

impl<'a> ProxyCostBackfillRequest<'a> {
    pub(crate) fn new(
        start_after_id: i64,
        snapshot_max_id: i64,
        catalog: &'a PricingCatalog,
        price_versions: (&'a str, &'a str, &'a str),
        scan_limit: Option<u64>,
        max_elapsed: Option<Duration>,
    ) -> Self {
        Self {
            start_after_id,
            snapshot_max_id,
            catalog,
            attempt_version: price_versions.0,
            requested_tier_price_version: price_versions.1,
            response_tier_price_version: price_versions.2,
            scan_limit,
            max_elapsed,
        }
    }
}

pub(crate) async fn run_proxy_cost_backfill(
    state: &AppState,
    cursor_id: i64,
    scan_limit: u64,
    max_elapsed: Option<Duration>,
) -> Result<BackfillBatchOutcome<ProxyCostBackfillSummary>> {
    let catalog = state.pricing_catalog.read().await.clone();
    let attempt_version = pricing_backfill_attempt_version(&catalog);
    let requested_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::RequestedTier);
    let response_tier_price_version =
        proxy_price_version(&catalog.version, ProxyPricingMode::ResponseTier);
    let snapshot_max_id = current_proxy_cost_backfill_snapshot_max_id(
        &state.pool,
        &attempt_version,
        &requested_tier_price_version,
        &response_tier_price_version,
    )
    .await?;
    backfill_proxy_missing_costs_from_cursor(
        &state.pool,
        ProxyCostBackfillRequest::new(
            cursor_id,
            snapshot_max_id,
            &catalog,
            (
                &attempt_version,
                &requested_tier_price_version,
                &response_tier_price_version,
            ),
            Some(scan_limit),
            max_elapsed,
        ),
    )
    .await
}

pub(crate) async fn backfill_proxy_missing_costs_from_cursor(
    pool: &Pool<Sqlite>,
    request: ProxyCostBackfillRequest<'_>,
) -> Result<BackfillBatchOutcome<ProxyCostBackfillSummary>> {
    let started_at = Instant::now();
    let mut summary = ProxyCostBackfillSummary::default();
    let mut last_seen_id = request.start_after_id;
    let mut hit_budget = false;
    let mut samples = Vec::new();

    loop {
        if startup_backfill_budget_reached(
            started_at,
            summary.scanned,
            request.scan_limit,
            request.max_elapsed,
        ) {
            hit_budget = true;
            break;
        }

        let mut pending_updates = Vec::new();
        let candidates = sqlx::query_as::<_, ProxyCostBackfillCandidate>(PROXY_COST_BACKFILL_QUERY)
            .bind(SOURCE_PROXY)
            .bind(last_seen_id)
            .bind(request.snapshot_max_id)
            .bind(request.attempt_version)
            .bind(request.requested_tier_price_version)
            .bind(API_KEYS_BILLING_ACCOUNT_KIND)
            .bind(request.response_tier_price_version)
            .bind(startup_backfill_query_limit(
                summary.scanned,
                request.scan_limit,
            ))
            .fetch_all(pool)
            .await?;

        if candidates.is_empty() {
            break;
        }

        for candidate in candidates {
            last_seen_id = candidate.id;
            summary.scanned += 1;
            if let Some(update) = build_proxy_cost_backfill_update(
                &candidate,
                request.catalog,
                request.attempt_version,
                &mut summary,
                &mut samples,
            ) {
                pending_proxy_cost_update(&mut pending_updates, update);
            }
        }

        let updated = apply_proxy_cost_backfill_updates(pool, pending_updates).await?;
        summary.updated += updated;
    }

    Ok(BackfillBatchOutcome {
        summary,
        next_cursor_id: last_seen_id,
        hit_budget,
        samples,
    })
}

const PROXY_COST_BACKFILL_QUERY: &str = r#"
WITH raw AS (
    SELECT
        inv.id,
        inv.model,
        inv.input_tokens,
        inv.output_tokens,
        inv.cache_input_tokens,
        inv.reasoning_tokens,
        inv.total_tokens,
        inv.cost,
        inv.price_version,
        CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requestedServiceTier') = 'text'
            THEN json_extract(inv.payload, '$.requestedServiceTier')
            WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.requested_service_tier') = 'text'
            THEN json_extract(inv.payload, '$.requested_service_tier') END AS requested_service_tier,
        CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.serviceTier') = 'text'
            THEN json_extract(inv.payload, '$.serviceTier')
            WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.service_tier') = 'text'
            THEN json_extract(inv.payload, '$.service_tier') END AS service_tier,
        CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billingServiceTier') = 'text'
            THEN json_extract(inv.payload, '$.billingServiceTier')
            WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.billing_service_tier') = 'text'
            THEN json_extract(inv.payload, '$.billing_service_tier') END AS billing_service_tier,
        CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamAccountKind') = 'text'
            THEN json_extract(inv.payload, '$.upstreamAccountKind')
            WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_account_kind') = 'text'
            THEN json_extract(inv.payload, '$.upstream_account_kind') END AS snapshot_upstream_account_kind,
        CASE WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstreamBaseUrlHost') = 'text'
            THEN json_extract(inv.payload, '$.upstreamBaseUrlHost')
            WHEN json_valid(inv.payload) AND json_type(inv.payload, '$.upstream_base_url_host') = 'text'
            THEN json_extract(inv.payload, '$.upstream_base_url_host') END AS snapshot_upstream_base_url_host,
        acc.kind AS live_upstream_account_kind,
        CASE WHEN acc.upstream_base_url IS NULL OR TRIM(CAST(acc.upstream_base_url AS TEXT)) = '' THEN NULL
            ELSE CASE WHEN INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') > 0
                THEN SUBSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), 1,
                    INSTR(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') - 1)
                ELSE RTRIM(REPLACE(REPLACE(LOWER(TRIM(CAST(acc.upstream_base_url AS TEXT))), 'https://', ''), 'http://', ''), '/') END END AS live_base_url_raw,
        CASE WHEN acc.created_at IS NOT NULL AND TRIM(CAST(acc.created_at AS TEXT)) != ''
            AND inv.occurred_at IS NOT NULL AND TRIM(CAST(inv.occurred_at AS TEXT)) != ''
            AND julianday(acc.created_at) <= julianday(inv.occurred_at)
            AND (acc.updated_at IS NULL OR TRIM(CAST(acc.updated_at AS TEXT)) = ''
                OR julianday(acc.updated_at) <= julianday(inv.occurred_at)) THEN 1 ELSE 0 END AS live_upstream_account_snapshot_safe
    FROM codex_invocations inv
    LEFT JOIN pool_upstream_accounts acc
      ON acc.id = CASE WHEN json_valid(inv.payload)
        THEN CAST(json_extract(inv.payload, '$.upstreamAccountId') AS INTEGER) END
    WHERE inv.source = ?1
      AND LOWER(TRIM(COALESCE(inv.status, ''))) IN ('success', 'warning_success', 'failed')
      AND inv.model IS NOT NULL
      AND (COALESCE(inv.input_tokens, 0) > 0 OR COALESCE(inv.output_tokens, 0) > 0
        OR COALESCE(inv.cache_input_tokens, 0) > 0 OR COALESCE(inv.reasoning_tokens, 0) > 0)
      AND inv.id > ?2 AND inv.id <= ?3
),
base AS (
    SELECT raw.*,
        CASE WHEN INSTR(live_base_url_raw, ':') > 0
            THEN SUBSTR(live_base_url_raw, 1, INSTR(live_base_url_raw, ':') - 1)
            ELSE live_base_url_raw END AS live_upstream_base_url_host
    FROM raw
),
cost_candidates AS (
    SELECT base.*,
        CASE WHEN LOWER(TRIM(COALESCE(snapshot_upstream_account_kind,
            CASE WHEN live_upstream_account_snapshot_safe = 1 THEN live_upstream_account_kind END, ''))) = ?6
            AND TRIM(COALESCE(requested_service_tier, '')) != '' THEN 1 ELSE 0 END
            AS uses_requested_tier_strategy
    FROM base
)
SELECT id, model, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens,
    requested_service_tier, service_tier, snapshot_upstream_account_kind, snapshot_upstream_base_url_host,
    live_upstream_base_url_host, live_upstream_account_kind, live_upstream_account_snapshot_safe
FROM cost_candidates
WHERE (uses_requested_tier_strategy = 1
    AND (LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(requested_service_tier, '')))
        OR (cost IS NULL AND (price_version IS NULL OR price_version != ?4))
        OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?5))))
 OR (uses_requested_tier_strategy = 0
    AND (LOWER(TRIM(COALESCE(billing_service_tier, ''))) != LOWER(TRIM(COALESCE(service_tier, '')))
        OR (cost IS NULL AND (price_version IS NULL OR price_version != ?4))
        OR (cost IS NOT NULL AND (price_version IS NULL OR price_version != ?7))))
ORDER BY id ASC
LIMIT ?8
"#;

fn build_proxy_cost_backfill_update(
    candidate: &ProxyCostBackfillCandidate,
    catalog: &PricingCatalog,
    attempt_version: &str,
    summary: &mut ProxyCostBackfillSummary,
    samples: &mut Vec<String>,
) -> Option<ProxyCostBackfillUpdate> {
    let Some(model) = candidate.model.as_deref() else {
        summary.skipped_unpriced_model += 1;
        return None;
    };
    let usage = ParsedUsage {
        input_tokens: candidate.input_tokens,
        output_tokens: candidate.output_tokens,
        cache_input_tokens: candidate.cache_input_tokens,
        reasoning_tokens: candidate.reasoning_tokens,
        total_tokens: candidate.total_tokens,
    };
    if !has_billable_usage(&usage) {
        summary.skipped_unpriced_model += 1;
        return None;
    }
    let allow_live_fallback =
        allow_live_upstream_account_fallback(Some(candidate.live_upstream_account_snapshot_safe));
    let upstream_account_kind = resolve_backfill_upstream_account_kind(
        candidate.snapshot_upstream_account_kind.as_deref(),
        candidate.live_upstream_account_kind.as_deref(),
        allow_live_fallback,
    );
    let upstream_base_url_host = resolve_backfill_upstream_base_url_host(
        candidate.snapshot_upstream_base_url_host.as_deref(),
        candidate.live_upstream_base_url_host.as_deref(),
        allow_live_fallback,
    );
    let (billing_service_tier, pricing_mode) = resolve_proxy_billing_service_tier_and_pricing_mode(
        None,
        candidate.requested_service_tier.as_deref(),
        candidate.service_tier.as_deref(),
        upstream_account_kind.as_deref(),
    );
    let (cost, cost_estimated, price_version) = estimate_proxy_cost(
        catalog,
        Some(model),
        &usage,
        billing_service_tier.as_deref(),
        pricing_mode,
    );
    if cost.is_none() || !cost_estimated {
        summary.skipped_unpriced_model += 1;
        push_backfill_sample(
            samples,
            format!("id={} model={} reason=unpriced_model", candidate.id, model),
        );
    }
    let persisted_price_version = if cost_estimated && cost.is_some() {
        price_version
    } else {
        Some(attempt_version.to_string())
    };
    Some(ProxyCostBackfillUpdate {
        id: candidate.id,
        cost,
        cost_estimated,
        price_version: persisted_price_version,
        billing_service_tier,
        upstream_account_kind,
        upstream_base_url_host,
    })
}

fn pending_proxy_cost_update(
    updates: &mut Vec<ProxyCostBackfillUpdate>,
    update: ProxyCostBackfillUpdate,
) {
    updates.push(update);
}

async fn apply_proxy_cost_backfill_updates(
    pool: &Pool<Sqlite>,
    updates: Vec<ProxyCostBackfillUpdate>,
) -> Result<u64> {
    if updates.is_empty() {
        return Ok(0);
    }
    let mut tx = pool.begin().await?;
    let mut updated = 0_u64;
    let mut updated_ids = Vec::new();
    for update in updates {
        let affected = sqlx::query(
            "UPDATE codex_invocations SET payload = json_set(json_set(json_set(CASE WHEN json_valid(payload) THEN payload ELSE '{}' END, '$.billingServiceTier', ?1), '$.upstreamAccountKind', ?2), '$.upstreamBaseUrlHost', ?3), cost = ?4, cost_estimated = ?5, price_version = ?6 WHERE id = ?7 AND source = ?8",
        )
        .bind(update.billing_service_tier.as_deref())
        .bind(update.upstream_account_kind.as_deref())
        .bind(update.upstream_base_url_host.as_deref())
        .bind(update.cost)
        .bind(update.cost_estimated as i64)
        .bind(update.price_version.as_deref())
        .bind(update.id)
        .bind(SOURCE_PROXY)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        updated += affected;
        if affected > 0 {
            updated_ids.push(update.id);
        }
    }
    if !updated_ids.is_empty() {
        recompute_invocation_hourly_rollups_for_ids_tx(tx.as_mut(), &updated_ids).await?;
    }
    tx.commit().await?;
    Ok(updated)
}
