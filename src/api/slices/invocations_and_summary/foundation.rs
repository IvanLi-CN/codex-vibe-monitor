pub(crate) const INVOCATION_PROXY_DISPLAY_SQL: &str = "NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), '')";
pub(crate) const INVOCATION_ENDPOINT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_REQUEST_KIND_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionRequestKind') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_RESPONSE_KIND_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionResponseKind') AS TEXT) END";
pub(crate) const INVOCATION_IMAGE_INTENT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.imageIntent') AS TEXT) END";
pub(crate) const INVOCATION_FAILURE_KIND_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind)";
pub(crate) const INVOCATION_REQUESTER_IP_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requesterIp') AS TEXT) END";
pub(crate) const INVOCATION_PROMPT_CACHE_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
pub(crate) const INVOCATION_STICKY_KEY_SQL: &str = "CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END";
pub(crate) const INVOCATION_UPSTREAM_SCOPE_SQL: &str = "COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamScope') AS TEXT) END, 'external')";
pub(crate) const INVOCATION_ROUTE_MODE_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.routeMode') AS TEXT) END";
pub(crate) const INVOCATION_REQUEST_MODEL_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requestModel') AS TEXT) END";
pub(crate) const INVOCATION_RESPONSE_MODEL_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.responseModel') AS TEXT) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL: &str = "COALESCE((SELECT NULLIF(TRIM(sample.plan_type), '') FROM pool_upstream_account_limit_samples sample WHERE sample.account_id = CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AND sample.plan_type IS NOT NULL AND TRIM(sample.plan_type) <> '' ORDER BY sample.captured_at DESC, sample.id DESC LIMIT 1), (SELECT NULLIF(TRIM(account.plan_type), '') FROM pool_upstream_accounts account WHERE account.id = CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END))";
pub(crate) const INVOCATION_REASONING_EFFORT_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.reasoningEffort') = 'text' THEN json_extract(payload, '$.reasoningEffort') END";
pub(crate) const INVOCATION_RESPONSE_CONTENT_ENCODING_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.responseContentEncoding') AS TEXT) END";
pub(crate) const INVOCATION_REQUEST_COMPRESSION_ALGORITHM_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requestCompressionAlgorithm') AS TEXT) END";
pub(crate) const INVOCATION_DOWNSTREAM_STATUS_CODE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamStatusCode') AS INTEGER) END";
pub(crate) const INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END";
pub(crate) const INVOCATION_TRANSPORT_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.transport') = 'text' THEN json_extract(payload, '$.transport') END";
pub(crate) const INVOCATION_SERVICE_TIER_SQL: &str = "CASE   WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text'     THEN json_extract(payload, '$.serviceTier')   WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text'     THEN json_extract(payload, '$.service_tier') END";
pub(crate) const INVOCATION_BILLING_SERVICE_TIER_SQL: &str = "CASE   WHEN json_valid(payload) AND json_type(payload, '$.billingServiceTier') = 'text'     THEN json_extract(payload, '$.billingServiceTier')   WHEN json_valid(payload) AND json_type(payload, '$.billing_service_tier') = 'text'     THEN json_extract(payload, '$.billing_service_tier') END";
pub(crate) const INVOCATION_POOL_ATTEMPT_COUNT_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolAttemptCount') AS INTEGER) END";
pub(crate) const INVOCATION_POOL_DISTINCT_ACCOUNT_COUNT_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolDistinctAccountCount') AS INTEGER) END";
pub(crate) const INVOCATION_POOL_ATTEMPT_TERMINAL_REASON_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.poolAttemptTerminalReason') AS TEXT) END";
pub(crate) const INVOCATION_BLOCKED_BINDING_JSON_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.blockedBinding') = 'object' THEN json_extract(payload, '$.blockedBinding') END";
pub(crate) const PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT: usize = 3;
pub(crate) const PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT: usize = 5;
pub(crate) const INVOCATION_STATUS_NORMALIZED_SQL: &str = "LOWER(TRIM(COALESCE(status, '')))";
pub(crate) const INVOCATION_RESPONSE_BODY_PREVIEW_CHAR_LIMIT: usize = 2_000;
pub(crate) const INVOCATION_LIVE_PHASE_QUEUED: &str = "queued";
pub(crate) const INVOCATION_LIVE_PHASE_REQUESTING: &str = "requesting";
pub(crate) const INVOCATION_LIVE_PHASE_RESPONDING: &str = "responding";
const INVOCATION_ANCHOR_TTL: Duration = Duration::from_secs(30 * 60);
const INVOCATION_ANCHOR_CACHE_LIMIT: usize = 32;

#[derive(Clone)]
struct InvocationAnchorSnapshot {
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
    runtime_records: Vec<ApiInvocation>,
    expires_at: Instant,
}

static INVOCATION_ANCHOR_SNAPSHOTS: once_cell::sync::Lazy<
    StdMutex<HashMap<String, InvocationAnchorSnapshot>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));

#[cfg(test)]
const SUMMARY_PROJECTION_TEST_INTERLEAVE_TABLE: &str = "summary_projection_test_interleave_gate";

#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SummaryProjectionTestInterleaveStage {
    AfterRollupLoad,
    AfterAllTimeArchiveDiscovery,
    BeforeAllTimeArchiveScan,
    BeforePagedBoundaryArchiveHydration,
    BeforeHistoricalLiveCoverage,
    BeforeProjectionPublication,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct SummaryProjectionTestInterleave {
    writer_ready: Arc<tokio::sync::Notify>,
    resume_build: Arc<tokio::sync::Notify>,
    build_attempts: Arc<AtomicUsize>,
    build_modes: Arc<StdMutex<Vec<SummaryProjectionBuildMode>>>,
    stages: Arc<[SummaryProjectionTestInterleaveStage]>,
    next_stage_index: Arc<AtomicUsize>,
}

#[cfg(test)]
static SUMMARY_PROJECTION_TEST_INTERLEAVE: once_cell::sync::Lazy<
    StdMutex<Option<SummaryProjectionTestInterleave>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(None));

#[cfg(test)]
impl SummaryProjectionTestInterleave {
    pub(crate) async fn wait_for_writer(&self) {
        self.writer_ready.notified().await;
    }

    pub(crate) fn resume_build(&self) {
        self.resume_build.notify_one();
    }

    pub(crate) fn build_attempts(&self) -> usize {
        self.build_attempts.load(Ordering::SeqCst)
    }

    pub(crate) fn build_modes(&self) -> Vec<SummaryProjectionBuildMode> {
        self.build_modes
            .lock()
            .expect("summary projection test interleave mode lock")
            .clone()
    }
}

#[cfg(test)]
pub(crate) fn install_summary_projection_test_interleave() -> SummaryProjectionTestInterleave {
    install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::AfterRollupLoad,
    )
}

#[cfg(test)]
pub(crate) fn install_summary_projection_test_interleave_at(
    stage: SummaryProjectionTestInterleaveStage,
) -> SummaryProjectionTestInterleave {
    install_summary_projection_test_interleave_for_stages(&[stage])
}

#[cfg(test)]
pub(crate) fn install_summary_projection_test_interleave_for_stages(
    stages: &[SummaryProjectionTestInterleaveStage],
) -> SummaryProjectionTestInterleave {
    assert!(
        !stages.is_empty(),
        "summary projection test interleave needs at least one stage"
    );
    let interleave = SummaryProjectionTestInterleave {
        writer_ready: Arc::new(tokio::sync::Notify::new()),
        resume_build: Arc::new(tokio::sync::Notify::new()),
        build_attempts: Arc::new(AtomicUsize::new(0)),
        build_modes: Arc::new(StdMutex::new(Vec::new())),
        stages: stages.into(),
        next_stage_index: Arc::new(AtomicUsize::new(0)),
    };
    let mut installed = SUMMARY_PROJECTION_TEST_INTERLEAVE
        .lock()
        .expect("summary projection test interleave lock");
    assert!(
        installed.is_none(),
        "summary projection test interleave is already installed"
    );
    *installed = Some(interleave.clone());
    interleave
}

#[cfg(test)]
pub(crate) fn clear_summary_projection_test_interleave() {
    *SUMMARY_PROJECTION_TEST_INTERLEAVE
        .lock()
        .expect("summary projection test interleave lock") = None;
}

#[cfg(test)]
async fn pause_summary_projection_test_interleave(
    pool: &Pool<Sqlite>,
    mode: SummaryProjectionBuildMode,
    stage: SummaryProjectionTestInterleaveStage,
) -> Result<()> {
    let table_present = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
    )
    .bind(SUMMARY_PROJECTION_TEST_INTERLEAVE_TABLE)
    .fetch_one(pool)
    .await?;
    if table_present == 0 {
        return Ok(());
    }
    let interleave = SUMMARY_PROJECTION_TEST_INTERLEAVE
        .lock()
        .expect("summary projection test interleave lock")
        .clone();
    let Some(interleave) = interleave else {
        return Ok(());
    };
    let next_stage_index = interleave.next_stage_index.load(Ordering::SeqCst);
    if interleave.stages.get(next_stage_index) != Some(&stage) {
        return Ok(());
    }
    interleave.next_stage_index.fetch_add(1, Ordering::SeqCst);
    interleave.build_attempts.fetch_add(1, Ordering::SeqCst);
    interleave
        .build_modes
        .lock()
        .expect("summary projection test interleave mode lock")
        .push(mode);
    interleave.writer_ready.notify_one();
    interleave.resume_build.notified().await;
    Ok(())
}

fn store_invocation_anchor_snapshot(
    snapshot_id: i64,
    upstream_account_id: Option<i64>,
    runtime_records: Vec<ApiInvocation>,
) -> String {
    let anchor_id = nanoid::nanoid!(16);
    let now = Instant::now();
    let mut snapshots = INVOCATION_ANCHOR_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    if snapshots.len() >= INVOCATION_ANCHOR_CACHE_LIMIT
        && let Some(oldest_id) = snapshots
            .iter()
            .min_by_key(|(_, snapshot)| snapshot.expires_at)
            .map(|(id, _)| id.clone())
    {
        snapshots.remove(&oldest_id);
    }
    snapshots.insert(
        anchor_id.clone(),
        InvocationAnchorSnapshot {
            snapshot_id,
            upstream_account_id,
            runtime_records,
            expires_at: now + INVOCATION_ANCHOR_TTL,
        },
    );
    anchor_id
}

fn load_invocation_anchor_runtime_records(
    params: &ListQuery,
) -> Result<Option<Vec<ApiInvocation>>, ApiError> {
    let Some(anchor_id) = normalize_query_text(params.anchor_id.as_deref()) else {
        return Ok(None);
    };
    let now = Instant::now();
    let mut snapshots = INVOCATION_ANCHOR_SNAPSHOTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    snapshots.retain(|_, snapshot| snapshot.expires_at > now);
    let snapshot = snapshots
        .get(&anchor_id)
        .ok_or_else(|| ApiError::bad_request(anyhow!("invocation anchor expired or not found")))?;
    if params.snapshot_id != Some(snapshot.snapshot_id)
        || params.upstream_account_id != snapshot.upstream_account_id
    {
        return Err(ApiError::bad_request(anyhow!(
            "invocation anchor does not match snapshot or account"
        )));
    }
    Ok(Some(snapshot.runtime_records.clone()))
}

// Legacy records can carry `failure_class=none` or NULL while still representing failures.
// Keep classification consistent with `resolve_failure_classification` without requiring a
// backfill pass to complete before the summary + filters become accurate.
pub(crate) const INVOCATION_RESOLVED_FAILURE_CLASS_SQL: &str = concat!(
    "CASE ",
    "  WHEN LOWER(TRIM(COALESCE(failure_class, ''))) IN ('service_failure', 'client_failure', 'client_abort') ",
    "    THEN LOWER(TRIM(COALESCE(failure_class, ''))) ",
    "  ELSE ",
    "    CASE ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed') ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = 'warning_success' ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending') ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'downstream_closed' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[downstream_closed]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%downstream closed while streaming upstream response%' ",
    "        OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) LIKE '%downstream closed while streaming upstream response%' ",
    "        THEN 'client_abort' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_429' ",
    "        OR LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = 'upstream_http_429' ",
    "        THEN 'service_failure' ",
    "      WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('request_body_stream_error_client_closed', 'invalid_api_key', 'api_key_not_found', 'api_key_missing') ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_stream_error_client_closed]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to read request body stream%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%invalid api key format%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key format is invalid%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%incorrect api key provided%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%api key not found%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%please provide an api key%' ",
    "        OR (LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_4%' AND LOWER(TRIM(COALESCE(status, ''))) != 'http_429') ",
    "        OR LOWER(TRIM(COALESCE(status, ''))) IN ('http_401', 'http_403') ",
    "        THEN 'client_failure' ",
    "      WHEN LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) IN ('failed_contact_upstream', 'upstream_response_failed', 'upstream_stream_error', 'request_body_read_timeout', 'upstream_handshake_timeout') ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[failed_contact_upstream]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_response_failed]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_stream_error]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[request_body_read_timeout]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '[upstream_handshake_timeout]%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%failed to contact upstream%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream response stream reported failure%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream stream error%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%request body read timed out%' ",
    "        OR LOWER(TRIM(COALESCE(error_message, ''))) LIKE '%upstream handshake timed out%' ",
    "        OR LOWER(TRIM(COALESCE(status, ''))) LIKE 'http_5%' ",
    "        THEN 'service_failure' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) IN ('success', 'completed', 'warning_success') ",
    "        THEN 'none' ",
    "      WHEN LOWER(TRIM(COALESCE(status, ''))) = 'http_200' ",
    "        AND LOWER(TRIM(COALESCE(error_message, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END, ''))) = '' ",
    "        AND LOWER(TRIM(COALESCE(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.failureKind') AS TEXT) END, failure_kind, ''))) = '' ",
    "        THEN 'none' ",
    "      ELSE 'service_failure' ",
    "    END ",
    "END"
);

fn invocation_resolved_failure_class_sql_for(invocation_ref: &str) -> String {
    // The shared expression is unqualified for its many single-table callers. This query joins
    // attempt rows, so every invocation column must be bound to the invocation alias.
    let mut sql = INVOCATION_RESOLVED_FAILURE_CLASS_SQL.to_string();
    for column in [
        "failure_class",
        "failure_kind",
        "error_message",
        "payload",
        "status",
    ] {
        sql = sql.replace(column, &format!("{invocation_ref}.{column}"));
    }
    sql
}

pub(crate) fn latest_pool_attempt_phase_sql(invocation_ref: &str) -> String {
    format!(
        "(SELECT LOWER(TRIM(COALESCE(attempt.phase, ''))) \
            FROM pool_upstream_request_attempts attempt \
           WHERE attempt.invoke_id = {invocation_ref}.invoke_id \
             AND attempt.occurred_at = {invocation_ref}.occurred_at \
           ORDER BY attempt.attempt_index DESC, attempt.id DESC \
           LIMIT 1)"
    )
}

pub(crate) fn final_pool_attempt_first_token_ms_sql(
    attempt_ref: &str,
    invocation_ref: &str,
) -> String {
    let first_token_sql =
        sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.first_token_ms"));
    let final_attempt_evidence_sql =
        final_pool_attempt_timing_evidence_sql(attempt_ref, invocation_ref, "first_token_ms");
    format!(
        "CASE WHEN {attempt_ref}.id = (SELECT final_attempt.id \
           FROM pool_upstream_request_attempts AS final_attempt \
           WHERE final_attempt.invoke_id = {attempt_ref}.invoke_id \
             AND final_attempt.occurred_at = {attempt_ref}.occurred_at \
             AND LOWER(TRIM(COALESCE(final_attempt.status, ''))) <> 'budget_exhausted_final' \
           ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC \
           LIMIT 1) \
          AND ({final_attempt_evidence_sql}) \
        THEN CASE WHEN {first_token_sql} THEN {invocation_ref}.first_token_ms END END AS first_token_ms"
    )
}

pub(crate) fn final_pool_invocation_timing_sql(invocation_ref: &str, column: &str) -> String {
    let timing_sql = match column {
        "first_token_ms" => sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.{column}")),
        "t_upstream_stream_ms" => sqlite_positive_timing_sql(&format!("{invocation_ref}.{column}")),
        _ => panic!("unsupported pool invocation timing column: {column}"),
    };
    let final_attempt_evidence_sql =
        final_pool_attempt_timing_evidence_sql("final_attempt", invocation_ref, column);
    format!(
        "CASE WHEN NOT EXISTS (SELECT 1 \
           FROM pool_upstream_request_attempts AS final_attempt \
          WHERE final_attempt.invoke_id = {invocation_ref}.invoke_id \
            AND final_attempt.occurred_at = {invocation_ref}.occurred_at) \
           OR COALESCE((SELECT CASE \
             WHEN ({final_attempt_evidence_sql}) THEN 1 \
             ELSE 0 END \
           FROM pool_upstream_request_attempts AS final_attempt \
          WHERE final_attempt.invoke_id = {invocation_ref}.invoke_id \
            AND final_attempt.occurred_at = {invocation_ref}.occurred_at \
            AND LOWER(TRIM(COALESCE(final_attempt.status, ''))) <> 'budget_exhausted_final' \
           ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC \
           LIMIT 1), 0) = 1 \
        THEN CASE WHEN {timing_sql} THEN {invocation_ref}.{column} END END"
    )
}

fn invocation_timing_sql_for_source(
    invocation_ref: &str,
    column: &str,
    use_attempt_fallback: bool,
) -> String {
    if use_attempt_fallback {
        return final_pool_invocation_timing_sql(invocation_ref, column);
    }

    let timing_sql = match column {
        "first_token_ms" => sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.{column}")),
        "t_upstream_stream_ms" => sqlite_positive_timing_sql(&format!("{invocation_ref}.{column}")),
        _ => panic!("unsupported invocation timing column: {column}"),
    };
    format!("CASE WHEN {timing_sql} THEN {invocation_ref}.{column} END")
}

fn final_pool_attempt_timing_evidence_sql(
    attempt_ref: &str,
    invocation_ref: &str,
    column: &str,
) -> String {
    let stream_measured_sql =
        sqlite_positive_timing_sql(&format!("{attempt_ref}.stream_latency_ms"));
    let zero_first_token_sql = if column == "first_token_ms" {
        let first_byte_sql =
            sqlite_nonnegative_timing_sql(&format!("{attempt_ref}.first_byte_latency_ms"));
        format!(
            " OR ({invocation_ref}.first_token_ms = 0 AND LOWER(TRIM(COALESCE({attempt_ref}.status, ''))) NOT IN ('', 'pending', 'running', 'budget_exhausted_final') AND {first_byte_sql} AND {attempt_ref}.first_byte_latency_ms = 0)"
        )
    } else {
        String::new()
    };
    let live_first_token_sql = if column == "first_token_ms" {
        let first_token_sql =
            sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.first_token_ms"));
        format!(
            " OR (LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) = 'running' AND {first_token_sql} AND (LOWER(TRIM(COALESCE({attempt_ref}.status, ''))) = 'responding' OR LOWER(TRIM(COALESCE({attempt_ref}.phase, ''))) IN ('responding', 'streaming_response')))"
        )
    } else {
        String::new()
    };
    let stream_evidence_sql = format!(
        "({stream_measured_sql} AND LOWER(TRIM(COALESCE({attempt_ref}.status, ''))) NOT IN ('', 'pending', 'running', 'budget_exhausted_final'))"
    );
    if column == "first_token_ms" {
        format!("{stream_evidence_sql}{zero_first_token_sql}{live_first_token_sql}")
    } else {
        stream_evidence_sql
    }
}

fn invocation_live_phase_sql_with_first_token_predicate(
    invocation_ref: &str,
    first_token_measured_sql: &str,
) -> String {
    let attempt_phase_sql = latest_pool_attempt_phase_sql(invocation_ref);
    let upstream_account_id_sql = format!(
        "CASE WHEN json_valid({invocation_ref}.payload) THEN CAST(json_extract({invocation_ref}.payload, '$.upstreamAccountId') AS INTEGER) END"
    );
    format!(
        "CASE \
           WHEN LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) NOT IN ('running', 'pending') THEN NULL \
           WHEN LOWER(TRIM(COALESCE({invocation_ref}.status, ''))) = 'pending' THEN '{queued}' \
           WHEN {first_token_measured_sql} THEN '{responding}' \
           WHEN {attempt_phase} IN ('connecting', 'sending_request', 'waiting_first_byte') \
             OR {upstream_account_id} IS NOT NULL \
             OR ({invocation_ref}.t_upstream_connect_ms IS NOT NULL AND {invocation_ref}.t_upstream_connect_ms > 0) \
             OR ({invocation_ref}.t_req_read_ms IS NOT NULL AND {invocation_ref}.t_req_read_ms > 0) \
             OR ({invocation_ref}.t_req_parse_ms IS NOT NULL AND {invocation_ref}.t_req_parse_ms > 0) THEN '{requesting}' \
           ELSE '{queued}' \
         END",
        attempt_phase = attempt_phase_sql,
        first_token_measured_sql = first_token_measured_sql,
        upstream_account_id = upstream_account_id_sql,
        queued = INVOCATION_LIVE_PHASE_QUEUED,
        requesting = INVOCATION_LIVE_PHASE_REQUESTING,
        responding = INVOCATION_LIVE_PHASE_RESPONDING,
    )
}

pub(crate) fn invocation_live_phase_sql(invocation_ref: &str) -> String {
    let first_token_measured_sql =
        sqlite_nonnegative_timing_sql(&format!("{invocation_ref}.first_token_ms"));
    invocation_live_phase_sql_with_first_token_predicate(invocation_ref, &first_token_measured_sql)
}

pub(crate) fn invocation_live_phase_sql_with_timing_sql(
    invocation_ref: &str,
    first_token_timing_sql: &str,
) -> String {
    let first_token_measured_sql =
        sqlite_nonnegative_timing_sql(&format!("({first_token_timing_sql})"));
    invocation_live_phase_sql_with_first_token_predicate(invocation_ref, &first_token_measured_sql)
}

fn has_positive_timing(values: &[Option<f64>]) -> bool {
    values
        .iter()
        .flatten()
        .any(|value| value.is_finite() && *value > 0.0)
}

fn sqlite_finite_timing_sql(column: &str) -> String {
    // SQLite does not expose an isfinite() predicate. For REAL infinity, x - x becomes NULL;
    // finite values keep a numeric zero, so this rejects non-finite persisted measurements. The
    // explicit numeric type guard is required because REAL-affinity columns can still contain
    // text such as "Infinity" or "NaN".
    format!("typeof({column}) IN ('integer', 'real') AND ({column} - {column}) IS NOT NULL")
}

fn sqlite_nonnegative_timing_sql(column: &str) -> String {
    format!(
        "{column} IS NOT NULL AND {column} >= 0 AND {}",
        sqlite_finite_timing_sql(column)
    )
}

fn sqlite_positive_timing_sql(column: &str) -> String {
    format!(
        "{column} IS NOT NULL AND {column} > 0 AND {}",
        sqlite_finite_timing_sql(column)
    )
}

fn has_measured_first_token(value: Option<f64>) -> bool {
    value.is_some_and(|value| value.is_finite() && value >= 0.0)
}

fn finite_nonnegative_timing(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

fn finite_positive_timing(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

fn final_attempt_status_is_terminal(status: &str) -> bool {
    !matches!(
        normalized_runtime_text(Some(status)).as_str(),
        "" | "pending" | "running" | "budget_exhausted_final"
    )
}

fn final_attempt_has_stream_evidence(attempt: &InvocationWorkflowAttemptRow) -> bool {
    final_attempt_status_is_terminal(&attempt.status)
        && finite_positive_timing(attempt.stream_latency_ms).is_some()
}

fn final_attempt_allows_zero_first_token(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
) -> bool {
    finite_nonnegative_timing(record.first_token_ms) == Some(0.0)
        && final_attempt_status_is_terminal(&attempt.status)
        && finite_nonnegative_timing(attempt.first_byte_latency_ms) == Some(0.0)
}

fn final_attempt_has_live_first_token_evidence(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
) -> bool {
    normalized_runtime_text(record.status.as_deref()) == "running"
        && has_measured_first_token(record.first_token_ms)
        && (matches!(
            normalized_runtime_text(Some(&attempt.status)).as_str(),
            "responding"
        ) || matches!(
            normalized_runtime_text(attempt.phase.as_deref()).as_str(),
            "responding" | "streaming_response"
        ))
}

fn final_attempt_has_first_token_evidence(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
) -> bool {
    final_attempt_has_stream_evidence(attempt)
        || final_attempt_allows_zero_first_token(record, attempt)
        || final_attempt_has_live_first_token_evidence(record, attempt)
}

pub(crate) fn runtime_invocation_live_phase(record: &ApiInvocation) -> Option<&'static str> {
    match normalized_runtime_text(record.status.as_deref()).as_str() {
        "pending" => Some(INVOCATION_LIVE_PHASE_QUEUED),
        "running" => {
            if has_measured_first_token(record.first_token_ms) {
                Some(INVOCATION_LIVE_PHASE_RESPONDING)
            } else if record.upstream_account_id.is_some()
                || has_positive_timing(&[
                    record.t_upstream_connect_ms,
                    record.t_req_read_ms,
                    record.t_req_parse_ms,
                ])
            {
                Some(INVOCATION_LIVE_PHASE_REQUESTING)
            } else {
                Some(INVOCATION_LIVE_PHASE_QUEUED)
            }
        }
        _ => None,
    }
}

pub(crate) fn effective_runtime_invocation_live_phase(
    record: &ApiInvocation,
) -> Option<&'static str> {
    if normalized_runtime_text(record.status.as_deref()) == "pending" {
        return Some(INVOCATION_LIVE_PHASE_QUEUED);
    }
    let inferred_phase = if runtime_record_is_retry(record)
        && record.id > 0
        && has_measured_first_token(record.first_token_ms)
    {
        let mut phase_record = record.clone();
        phase_record.first_token_ms = None;
        runtime_invocation_live_phase(&phase_record)?
    } else {
        runtime_invocation_live_phase(record)?
    };
    if inferred_phase == INVOCATION_LIVE_PHASE_RESPONDING {
        return Some(inferred_phase);
    }

    match normalized_runtime_text(record.live_phase.as_deref()).as_str() {
        INVOCATION_LIVE_PHASE_QUEUED => Some(INVOCATION_LIVE_PHASE_QUEUED),
        INVOCATION_LIVE_PHASE_REQUESTING => Some(INVOCATION_LIVE_PHASE_REQUESTING),
        _ => Some(inferred_phase),
    }
}
