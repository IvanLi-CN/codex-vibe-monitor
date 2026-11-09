use super::*;
pub(crate) const INVOCATION_PROXY_DISPLAY_SQL: &str = "NULLIF(TRIM(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.proxyDisplayName') AS TEXT) END), '')";
pub(crate) const INVOCATION_ENDPOINT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.endpoint') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_REQUEST_KIND_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionRequestKind') AS TEXT) END";
pub(crate) const INVOCATION_COMPACTION_RESPONSE_KIND_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.compactionResponseKind') AS TEXT) END";
pub(crate) const INVOCATION_IMAGE_INTENT_SQL: &str =
    "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.imageIntent') AS TEXT) END";
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
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.upstreamAccountId') = 'integer' THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) WHEN json_valid(payload) AND json_type(payload, '$.upstreamAccountId') = 'text' AND TRIM(CAST(json_extract(payload, '$.upstreamAccountId') AS TEXT)) <> '' AND (TRIM(CAST(json_extract(payload, '$.upstreamAccountId') AS TEXT)) NOT GLOB '*[^0-9]*' OR (SUBSTR(TRIM(CAST(json_extract(payload, '$.upstreamAccountId') AS TEXT)), 1, 1) IN ('-', '+') AND LENGTH(TRIM(CAST(json_extract(payload, '$.upstreamAccountId') AS TEXT))) > 1 AND SUBSTR(TRIM(CAST(json_extract(payload, '$.upstreamAccountId') AS TEXT)), 2) NOT GLOB '*[^0-9]*')) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_ID_INDEX_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT) END";
pub(crate) const INVOCATION_UPSTREAM_ACCOUNT_PLAN_TYPE_SQL: &str = "COALESCE((SELECT NULLIF(TRIM(sample.plan_type), '') FROM pool_upstream_account_limit_samples sample WHERE sample.account_id = CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AND sample.plan_type IS NOT NULL AND TRIM(sample.plan_type) <> '' ORDER BY sample.captured_at DESC, sample.id DESC LIMIT 1), (SELECT NULLIF(TRIM(account.plan_type), '') FROM pool_upstream_accounts account WHERE account.id = CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END))";
pub(crate) const INVOCATION_REASONING_EFFORT_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.reasoningEffort') = 'text' THEN json_extract(payload, '$.reasoningEffort') END";
pub(crate) const INVOCATION_RESPONSE_CONTENT_ENCODING_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.responseContentEncoding') AS TEXT) END";
pub(crate) const INVOCATION_REQUEST_COMPRESSION_ALGORITHM_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.requestCompressionAlgorithm') AS TEXT) END";
pub(crate) const INVOCATION_DOWNSTREAM_STATUS_CODE_SQL: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.downstreamStatusCode') AS INTEGER) END";
pub(crate) const INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL: &str = "CASE WHEN json_valid(payload) AND json_type(payload, '$.downstreamErrorMessage') = 'text' THEN CAST(json_extract(payload, '$.downstreamErrorMessage') AS TEXT) END";
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
pub(crate) const INVOCATION_ANCHOR_TTL: Duration = Duration::from_secs(30 * 60);
pub(crate) const INVOCATION_ANCHOR_CACHE_LIMIT: usize = 32;

#[derive(Clone)]
pub(crate) struct InvocationAnchorSnapshot {
    pub(crate) snapshot_id: i64,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) runtime_records: Vec<ApiInvocation>,
    pub(crate) expires_at: Instant,
}

pub(crate) static INVOCATION_ANCHOR_SNAPSHOTS: once_cell::sync::Lazy<
    StdMutex<HashMap<String, InvocationAnchorSnapshot>>,
> = once_cell::sync::Lazy::new(|| StdMutex::new(HashMap::new()));

#[cfg(test)]
pub(crate) const SUMMARY_PROJECTION_TEST_INTERLEAVE_TABLE: &str =
    "summary_projection_test_interleave_gate";
#[cfg(test)]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SummaryProjectionTestInterleaveStage {
    AfterRollupLoad,
    AfterAllTimeArchiveDiscovery,
    BeforeAllTimeArchiveScan,
    BeforePagedBoundaryArchiveHydration,
    BeforeArchiveIdentityRevalidation,
    BeforeHistoricalLiveCoverage,
    BeforeLiveTailPublication,
    BeforeProjectionPublication,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct SummaryProjectionTestInterleave {
    pub(crate) writer_ready: Arc<tokio::sync::Notify>,
    pub(crate) resume_build: Arc<tokio::sync::Notify>,
    pub(crate) build_attempts: Arc<AtomicUsize>,
    pub(crate) build_modes: Arc<StdMutex<Vec<SummaryProjectionBuildMode>>>,
    pub(crate) stages: Arc<[SummaryProjectionTestInterleaveStage]>,
    pub(crate) next_stage_index: Arc<AtomicUsize>,
}

#[cfg(test)]
pub(crate) static SUMMARY_PROJECTION_TEST_INTERLEAVE: once_cell::sync::Lazy<
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
pub(crate) async fn pause_summary_projection_test_interleave(
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
pub(crate) fn store_invocation_anchor_snapshot(
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

pub(crate) fn load_invocation_anchor_runtime_records(
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

pub(crate) fn has_positive_timing(values: &[Option<f64>]) -> bool {
    values
        .iter()
        .flatten()
        .any(|value| value.is_finite() && *value > 0.0)
}

pub(crate) fn sqlite_finite_timing_sql(column: &str) -> String {
    // SQLite does not expose an isfinite() predicate. For REAL infinity, x - x becomes NULL;
    // finite values keep a numeric zero, so this rejects non-finite persisted measurements. The
    // explicit numeric type guard is required because REAL-affinity columns can still contain
    // text such as "Infinity" or "NaN".
    format!("typeof({column}) IN ('integer', 'real') AND ({column} - {column}) IS NOT NULL")
}

pub(crate) fn sqlite_nonnegative_timing_sql(column: &str) -> String {
    format!(
        "{column} IS NOT NULL AND {column} >= 0 AND {}",
        sqlite_finite_timing_sql(column)
    )
}

pub(crate) fn sqlite_positive_timing_sql(column: &str) -> String {
    format!(
        "{column} IS NOT NULL AND {column} > 0 AND {}",
        sqlite_finite_timing_sql(column)
    )
}

pub(crate) fn has_measured_first_token(value: Option<f64>) -> bool {
    value.is_some_and(|value| value.is_finite() && value >= 0.0)
}

pub(crate) fn finite_nonnegative_timing(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

pub(crate) fn finite_positive_timing(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

pub(crate) fn final_attempt_status_is_terminal(status: &str) -> bool {
    !matches!(
        normalized_runtime_text(Some(status)).as_str(),
        "" | "pending" | "running" | "budget_exhausted_final"
    )
}

pub(crate) fn final_attempt_has_stream_evidence(attempt: &InvocationWorkflowAttemptRow) -> bool {
    final_attempt_status_is_terminal(&attempt.status)
        && finite_positive_timing(attempt.stream_latency_ms).is_some()
}

pub(crate) fn final_attempt_allows_zero_first_token(
    record: &ApiInvocation,
    attempt: &InvocationWorkflowAttemptRow,
) -> bool {
    finite_nonnegative_timing(record.first_token_ms) == Some(0.0)
        && final_attempt_status_is_terminal(&attempt.status)
        && finite_nonnegative_timing(attempt.first_byte_latency_ms) == Some(0.0)
}

pub(crate) fn final_attempt_has_live_first_token_evidence(
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

pub(crate) fn final_attempt_has_first_token_evidence(
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

#[cfg(test)]
mod invocation_live_phase_tests {
    use super::*;

    #[test]
    fn persisted_live_phase_requires_a_measured_first_token_to_be_responding() {
        let sql = invocation_live_phase_sql("invocation");

        assert!(sql.contains("invocation.first_token_ms IS NOT NULL"));
        assert!(sql.contains("invocation.first_token_ms >= 0"));
        assert!(!sql.contains("t_upstream_ttfb_ms IS NOT NULL"));
        assert!(!sql.contains("t_upstream_stream_ms IS NOT NULL"));
        assert!(!sql.contains("streaming_response"));
    }

    #[test]
    fn zero_millisecond_first_token_is_a_valid_measurement() {
        assert!(has_measured_first_token(Some(0.0)));
        assert!(!has_measured_first_token(Some(-0.1)));
        assert!(!has_measured_first_token(None));
    }

    #[test]
    fn final_attempt_first_token_sql_limits_invocation_timing_to_the_last_attempt() {
        let sql = final_pool_attempt_first_token_ms_sql("attempts", "inv");

        assert!(sql.contains("attempts.id"));
        assert!(sql.contains("final_attempt.invoke_id = attempts.invoke_id"));
        assert!(sql.contains("final_attempt.occurred_at = attempts.occurred_at"));
        assert!(!sql.contains("attempts.status, ''))) IN ('success', 'completed'"));
        assert!(sql.contains("attempts.stream_latency_ms > 0"));
        assert!(sql.contains("typeof(inv.first_token_ms) IN ('integer', 'real')"));
        assert!(sql.contains("inv.first_token_ms = 0"));
        assert!(sql.contains(
            "attempts.status, ''))) NOT IN ('', 'pending', 'running', 'budget_exhausted_final')"
        ));
        assert!(sql.contains("inv.status, ''))) = 'running' AND inv.first_token_ms IS NOT NULL"));
        assert!(sql.contains("attempts.status, ''))) = 'responding'"));
        assert!(sql.contains("attempts.phase, ''))) IN ('responding', 'streaming_response')"));
        assert!(!sql.contains("prior_attempt.attempt_index < attempts.attempt_index"));
        assert!(sql.contains("final_attempt.status, ''))) <> 'budget_exhausted_final'"));
        assert!(sql.contains("ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC"));
        assert!(sql.contains("THEN CASE WHEN inv.first_token_ms IS NOT NULL"));
        assert!(sql.ends_with("END END AS first_token_ms"));
    }

    #[test]
    fn final_attempt_timing_requires_stream_evidence_for_positive_values() {
        let first_token_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
        let stream_sql = final_pool_invocation_timing_sql("inv", "t_upstream_stream_ms");

        assert!(first_token_sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(stream_sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(!first_token_sql.contains("status, ''))) IN ('success', 'completed'"));
        assert!(!stream_sql.contains("status, ''))) IN ('success', 'completed'"));
    }

    #[test]
    fn invocation_timing_sql_requires_a_real_final_attempt_measurement() {
        let sql = final_pool_invocation_timing_sql("inv", "first_token_ms");

        assert!(sql.contains("final_attempt.first_byte_latency_ms"));
        assert!(sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(sql.contains("inv.first_token_ms = 0"));
        assert!(sql.contains("inv.first_token_ms >= 0"));
        assert!(sql.contains("ORDER BY final_attempt.attempt_index DESC, final_attempt.id DESC"));
        assert!(sql.contains("THEN inv.first_token_ms END END"));
    }

    #[test]
    fn archive_timing_sql_uses_validated_values_without_pool_attempts() {
        let first_token_sql = invocation_timing_sql_for_source("inv", "first_token_ms", false);
        let stream_sql = invocation_timing_sql_for_source("inv", "t_upstream_stream_ms", false);

        assert!(first_token_sql.contains("inv.first_token_ms >= 0"));
        assert!(stream_sql.contains("inv.t_upstream_stream_ms > 0"));
        assert!(!first_token_sql.contains("pool_upstream_request_attempts"));
        assert!(!stream_sql.contains("pool_upstream_request_attempts"));
    }

    #[test]
    fn persisted_live_phase_sql_uses_final_attempt_ttft() {
        let timing_sql = final_pool_invocation_timing_sql("inv", "first_token_ms");
        let sql = invocation_live_phase_sql_with_timing_sql("inv", &timing_sql);

        assert!(sql.contains("final_attempt.stream_latency_ms > 0"));
        assert!(sql.contains("final_attempt.first_byte_latency_ms"));
        assert!(sql.contains("THEN inv.first_token_ms END END"));
    }
}

pub(crate) fn normalized_runtime_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

pub(crate) const USAGE_BREAKDOWN_UNASSIGNED_ACCOUNT_KEY: &str = "__unassigned__";

pub(crate) fn usage_breakdown_payload_text(payload: Option<&str>, key: &str) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn normalized_usage_breakdown_model(
    model: Option<&str>,
    payload: Option<&str>,
) -> String {
    usage_breakdown_payload_text(payload, "responseModel")
        .or_else(|| {
            model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

pub(crate) fn normalized_usage_breakdown_reasoning_effort(payload: Option<&str>) -> Option<String> {
    usage_breakdown_payload_text(payload, "reasoningEffort")
}

pub(crate) fn normalized_usage_breakdown_account_key(
    payload: Option<&str>,
    upstream_account_id: Option<i64>,
) -> (String, Option<i64>) {
    match upstream_account_id.or_else(|| crate::proxy::upstream_account_id_from_payload(payload)) {
        Some(upstream_account_id) => (
            format!("upstream:{upstream_account_id}"),
            Some(upstream_account_id),
        ),
        None => (USAGE_BREAKDOWN_UNASSIGNED_ACCOUNT_KEY.to_string(), None),
    }
}

pub(crate) fn runtime_text_equals(value: Option<&str>, expected: &str) -> bool {
    normalized_runtime_text(value) == expected.trim().to_lowercase()
}

pub(crate) fn runtime_text_list_contains(values: &[String], candidate: Option<&str>) -> bool {
    let normalized_candidate = normalized_runtime_text(candidate);
    !normalized_candidate.is_empty()
        && values
            .iter()
            .any(|value| normalized_runtime_text(Some(value.as_str())) == normalized_candidate)
}

pub(crate) fn runtime_keyword_matches(record: &ApiInvocation, keyword: &str) -> bool {
    let keyword = keyword.trim().to_lowercase();
    if keyword.is_empty() {
        return true;
    }
    [
        Some(record.invoke_id.as_str()),
        record.model.as_deref(),
        record.request_model.as_deref(),
        record.response_model.as_deref(),
        record.proxy_display_name.as_deref(),
        record.endpoint.as_deref(),
        record.failure_kind.as_deref(),
        record.error_message.as_deref(),
        record.downstream_error_message.as_deref(),
        record.prompt_cache_key.as_deref(),
        record.requester_ip.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&keyword))
}

pub(crate) fn runtime_sticky_key(record: &ApiInvocation) -> Option<&str> {
    record
        .sticky_key
        .as_deref()
        .or(record.prompt_cache_key.as_deref())
}

pub(crate) fn runtime_upstream_scope(record: &ApiInvocation) -> &'static str {
    if runtime_text_equals(record.route_mode.as_deref(), "pool") {
        "internal"
    } else {
        "external"
    }
}

pub(crate) fn runtime_model_value(
    record: &ApiInvocation,
    target: InvocationModelTarget,
) -> Option<&str> {
    match target {
        InvocationModelTarget::Request => {
            record.request_model.as_deref().or(record.model.as_deref())
        }
        InvocationModelTarget::Response => {
            record.response_model.as_deref().or(record.model.as_deref())
        }
    }
}

pub(crate) fn runtime_model_rerouted(record: &ApiInvocation) -> bool {
    let request_model = normalized_runtime_text(record.request_model.as_deref());
    let response_model = normalized_runtime_text(record.response_model.as_deref());
    !request_model.is_empty() && !response_model.is_empty() && request_model != response_model
}

pub(crate) fn runtime_record_is_retry(record: &ApiInvocation) -> bool {
    record.pool_attempt_count.unwrap_or(1) > 1
}

pub(crate) fn runtime_record_first_token_ms(record: &ApiInvocation) -> Option<f64> {
    runtime_record_first_token_ms_with_retry(record, runtime_record_is_retry(record))
}

pub(crate) fn runtime_record_first_token_ms_with_retry(
    record: &ApiInvocation,
    is_retry: bool,
) -> Option<f64> {
    let first_token_ms = finite_nonnegative_timing(record.first_token_ms);
    if !is_retry {
        return first_token_ms;
    }

    // Runtime capture records use a synthetic row id and are rebuilt for the selected attempt.
    // Persisted rows need the attempt-table predicate instead of invocation-level timing.
    if record.id > 0 {
        return None;
    }
    if runtime_record_is_in_flight(record) {
        return (runtime_invocation_live_phase(record) == Some(INVOCATION_LIVE_PHASE_RESPONDING))
            .then_some(first_token_ms)
            .flatten();
    }

    (runtime_record_is_success_for_summary(record)
        || finite_positive_timing(record.t_upstream_stream_ms).is_some())
    .then_some(first_token_ms)
    .flatten()
}

pub(crate) fn runtime_record_live_phase(record: &ApiInvocation) -> Option<&'static str> {
    runtime_record_live_phase_with_retry(record, runtime_record_is_retry(record))
}

pub(crate) fn runtime_record_live_phase_with_retry(
    record: &ApiInvocation,
    is_retry: bool,
) -> Option<&'static str> {
    if is_retry && !runtime_record_is_retry(record) {
        let mut phase_record = record.clone();
        phase_record.pool_attempt_count = Some(2);
        effective_runtime_invocation_live_phase(&phase_record)
    } else {
        effective_runtime_invocation_live_phase(record)
    }
}

pub(crate) fn runtime_record_is_in_flight(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "running" | "pending"
    )
}

pub(crate) fn runtime_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
        return false;
    }
    if let Some(from_bound) = filters.occurred_from.as_deref()
        && record.occurred_at.as_str() < from_bound
    {
        return false;
    }
    if let Some(to_bound) = filters.occurred_to.as_deref()
        && record.occurred_at.as_str() >= to_bound
    {
        return false;
    }
    if !filters.model_values.is_empty() {
        let target = filters
            .model_target
            .unwrap_or(InvocationModelTarget::Request);
        if !runtime_text_list_contains(&filters.model_values, runtime_model_value(record, target)) {
            return false;
        }
    } else if let Some(model) = filters.model.as_deref()
        && !runtime_text_equals(record.model.as_deref(), model)
    {
        return false;
    }
    if let Some(model_rerouted) = filters.model_rerouted {
        let rerouted = runtime_model_rerouted(record);
        match model_rerouted {
            InvocationModelRerouteFilter::Rerouted if !rerouted => return false,
            InvocationModelRerouteFilter::NotRerouted if rerouted => return false,
            _ => {}
        }
    }
    if let Some(status) = filters.status.as_deref() {
        let normalized_status = status.trim();
        if normalized_status.eq_ignore_ascii_case("failed") {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable.map(|value| if value { 1 } else { 0 }),
            );
            if !prompt_cache_and_timeseries_shared::prompt_invocation_status_counts_toward_terminal_totals(
                record.status.as_deref(),
            ) || classification.failure_class == FailureClass::None
                || runtime_text_equals(record.status.as_deref(), "interrupted")
            {
                return false;
            }
        } else if normalized_status.eq_ignore_ascii_case("success") {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable.map(|value| if value { 1 } else { 0 }),
            );
            if !runtime_text_equals(record.status.as_deref(), normalized_status)
                || classification.failure_class != FailureClass::None
            {
                return false;
            }
        } else if normalized_status.eq_ignore_ascii_case(INVOCATION_STATUS_WARNING_SUCCESS) {
            let classification = resolve_failure_classification(
                record.status.as_deref(),
                record.error_message.as_deref(),
                record.failure_kind.as_deref(),
                record.failure_class.as_deref(),
                record.is_actionable.map(|value| if value { 1 } else { 0 }),
            );
            if !runtime_text_equals(record.status.as_deref(), normalized_status)
                || classification.failure_class != FailureClass::None
            {
                return false;
            }
        } else if !runtime_text_equals(record.status.as_deref(), normalized_status) {
            return false;
        }
    }
    if let Some(endpoint) = filters.endpoint.as_deref()
        && !runtime_text_equals(record.endpoint.as_deref(), endpoint)
    {
        return false;
    }
    if let Some(request_id) = filters.request_id.as_deref()
        && !runtime_text_equals(Some(record.invoke_id.as_str()), request_id)
    {
        return false;
    }
    if let Some(failure_class) = filters.failure_class.as_deref()
        && !runtime_text_equals(record.failure_class.as_deref(), failure_class)
    {
        return false;
    }
    if let Some(failure_kind) = filters.failure_kind.as_deref()
        && !runtime_text_equals(record.failure_kind.as_deref(), failure_kind)
    {
        return false;
    }
    if let Some(prompt_cache_key) = filters.prompt_cache_key.as_deref()
        && !runtime_text_equals(record.prompt_cache_key.as_deref(), prompt_cache_key)
    {
        return false;
    }
    if let Some(sticky_key) = filters.sticky_key.as_deref()
        && !runtime_text_equals(runtime_sticky_key(record), sticky_key)
    {
        return false;
    }
    if let Some(upstream_scope) = filters.upstream_scope.as_deref()
        && !runtime_text_equals(Some(runtime_upstream_scope(record)), upstream_scope)
    {
        return false;
    }
    if let Some(proxy_display_name) = filters.proxy_display_name.as_deref()
        && !runtime_text_equals(record.proxy_display_name.as_deref(), proxy_display_name)
    {
        return false;
    }
    if let Some(upstream_account_id) = filters.upstream_account_id
        && record.upstream_account_id != Some(upstream_account_id)
    {
        return false;
    }
    if let Some(transport) = filters.transport.as_deref()
        && !runtime_text_equals(record.transport.as_deref(), transport)
    {
        return false;
    }
    if let Some(service_tier) = filters.service_tier.as_deref()
        && !runtime_text_equals(record.service_tier.as_deref(), service_tier)
    {
        return false;
    }
    if !filters.reasoning_effort_values.is_empty() {
        if !runtime_text_list_contains(
            &filters.reasoning_effort_values,
            record.reasoning_effort.as_deref(),
        ) {
            return false;
        }
    } else if let Some(reasoning_effort) = filters.reasoning_effort.as_deref()
        && !runtime_text_equals(record.reasoning_effort.as_deref(), reasoning_effort)
    {
        return false;
    }
    if let Some(requester_ip) = filters.requester_ip.as_deref()
        && !runtime_text_equals(record.requester_ip.as_deref(), requester_ip)
    {
        return false;
    }
    if let Some(keyword) = filters.keyword.as_deref()
        && !runtime_keyword_matches(record, keyword)
    {
        return false;
    }
    if let Some(min_total_tokens) = filters.min_total_tokens {
        let Some(total_tokens) = record.total_tokens else {
            return false;
        };
        if total_tokens < min_total_tokens {
            return false;
        }
    }
    if let Some(max_total_tokens) = filters.max_total_tokens {
        let Some(total_tokens) = record.total_tokens else {
            return false;
        };
        if total_tokens > max_total_tokens {
            return false;
        }
    }
    if let Some(min_total_ms) = filters.min_total_ms {
        let Some(total_ms) = record.t_total_ms else {
            return false;
        };
        if total_ms < min_total_ms {
            return false;
        }
    }
    if let Some(max_total_ms) = filters.max_total_ms {
        let Some(total_ms) = record.t_total_ms else {
            return false;
        };
        if total_ms > max_total_ms {
            return false;
        }
    }
    true
}

pub(crate) fn runtime_in_flight_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    runtime_record_is_in_flight(record)
        && runtime_record_matches_filters(record, filters, source_scope)
}

pub(crate) fn option_presence_order(
    left_some: bool,
    right_some: bool,
) -> Option<std::cmp::Ordering> {
    match (left_some, right_some) {
        (true, false) => Some(std::cmp::Ordering::Less),
        (false, true) => Some(std::cmp::Ordering::Greater),
        (false, false) => Some(std::cmp::Ordering::Equal),
        (true, true) => None,
    }
}

pub(crate) fn apply_runtime_sort_order(
    ordering: std::cmp::Ordering,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    match sort_order {
        InvocationSortOrder::Asc => ordering,
        InvocationSortOrder::Desc => ordering.reverse(),
    }
}

pub(crate) fn compare_runtime_option_i64(
    left: Option<i64>,
    right: Option<i64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default().cmp(&right.unwrap_or_default()),
            sort_order,
        )
    })
}

pub(crate) fn compare_runtime_option_f64(
    left: Option<f64>,
    right: Option<f64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default()
                .partial_cmp(&right.unwrap_or_default())
                .unwrap_or(std::cmp::Ordering::Equal),
            sort_order,
        )
    })
}

pub(crate) fn compare_runtime_option_str(
    left: Option<&str>,
    right: Option<&str>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default().cmp(right.unwrap_or_default()),
            sort_order,
        )
    })
}

pub(crate) fn invocation_display_status_value(record: &ApiInvocation) -> Option<&str> {
    let status = record.status.as_deref().map(str::trim).unwrap_or_default();
    let failure_class = record
        .failure_class
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if status.eq_ignore_ascii_case("interrupted") {
        Some("interrupted")
    } else if matches!(
        failure_class,
        "service_failure" | "client_failure" | "client_abort"
    ) {
        Some("failed")
    } else if status.is_empty() {
        Some("unknown")
    } else {
        record.status.as_deref()
    }
}

pub(crate) fn compare_runtime_invocation_records(
    left: &ApiInvocation,
    right: &ApiInvocation,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    let primary = match sort_by {
        InvocationSortBy::OccurredAt => compare_runtime_option_str(
            Some(left.occurred_at.as_str()),
            Some(right.occurred_at.as_str()),
            sort_order,
        )
        .then_with(|| apply_runtime_sort_order(left.id.cmp(&right.id), sort_order)),
        InvocationSortBy::TotalTokens => {
            compare_runtime_option_i64(left.total_tokens, right.total_tokens, sort_order)
        }
        InvocationSortBy::Cost => compare_runtime_option_f64(left.cost, right.cost, sort_order),
        InvocationSortBy::TotalMs => {
            compare_runtime_option_f64(left.t_total_ms, right.t_total_ms, sort_order)
        }
        InvocationSortBy::TtfbMs => compare_runtime_option_f64(
            left.t_upstream_ttfb_ms,
            right.t_upstream_ttfb_ms,
            sort_order,
        ),
        InvocationSortBy::Status => compare_runtime_option_str(
            invocation_display_status_value(left),
            invocation_display_status_value(right),
            sort_order,
        ),
    };
    primary
        .then_with(|| right.occurred_at.cmp(&left.occurred_at))
        .then_with(|| right.id.cmp(&left.id))
}

pub(crate) fn should_overlay_runtime_records(request: &InvocationListRequest) -> bool {
    request.snapshot_id.is_none()
}

pub(crate) fn runtime_overlay_snapshot(state: &AppState) -> Vec<ApiInvocation> {
    state.proxy_runtime_invocations.snapshot()
}

pub(crate) async fn query_current_runtime_db_keys(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    let mut connection = pool.acquire().await?;
    query_current_runtime_db_keys_on_connection(&mut connection, filters, source_scope, snapshot)
        .await
}

pub(crate) async fn query_current_runtime_db_keys_on_connection(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT invoke_id, occurred_at FROM codex_invocations WHERE 1 = 1",
    );
    apply_invocation_records_filters(&mut query, filters, source_scope, snapshot);
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");

    Ok(query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(connection)
        .await?
        .into_iter()
        .map(|row| (row.invoke_id, row.occurred_at))
        .collect())
}

pub(crate) async fn count_stale_runtime_db_rows_before_target(
    pool: &Pool<Sqlite>,
    stale_keys: &HashSet<(String, String)>,
    target_occurred_at: &str,
    target_id: i64,
    snapshot_id: i64,
) -> Result<i64, ApiError> {
    #[derive(Debug, FromRow)]
    struct CountRow {
        total: i64,
    }

    let mut count = stale_keys
        .iter()
        .filter(|(_, occurred_at)| occurred_at.as_str() > target_occurred_at)
        .count() as i64;
    let equal_time_keys = stale_keys
        .iter()
        .filter(|(_, occurred_at)| occurred_at.as_str() == target_occurred_at)
        .collect::<Vec<_>>();
    for chunk in equal_time_keys.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(*) AS total FROM codex_invocations WHERE id <= ",
        );
        query
            .push_bind(snapshot_id)
            .push(" AND id > ")
            .push_bind(target_id)
            .push(" AND occurred_at = ")
            .push_bind(target_occurred_at.to_string())
            .push(" AND (");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        count += query
            .build_query_as::<CountRow>()
            .fetch_one(pool)
            .await?
            .total;
    }
    Ok(count)
}

pub(crate) async fn query_terminal_db_keys_for_runtime_records(
    pool: &Pool<Sqlite>,
    runtime_records: &[ApiInvocation],
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    let mut connection = pool.acquire().await?;
    query_terminal_db_keys_for_runtime_records_on_connection(
        &mut connection,
        runtime_records,
        snapshot,
    )
    .await
}

pub(crate) async fn query_terminal_db_keys_for_runtime_records_on_connection(
    connection: &mut SqliteConnection,
    runtime_records: &[ApiInvocation],
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
    }

    let keys = runtime_records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return Ok(HashSet::new());
    }

    let mut terminal_keys = HashSet::new();
    for chunk in keys.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT invoke_id, occurred_at FROM codex_invocations \
             WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') \
             AND (",
        );
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        if let Some(snapshot_constraint) = snapshot {
            match snapshot_constraint {
                SnapshotConstraint::UpTo(snapshot_id) => {
                    query.push(" AND id <= ").push_bind(snapshot_id);
                }
                SnapshotConstraint::After(snapshot_id) => {
                    query.push(" AND id > ").push_bind(snapshot_id);
                }
            }
        }
        terminal_keys.extend(
            query
                .build_query_as::<RuntimeKeyRow>()
                .fetch_all(&mut *connection)
                .await?
                .into_iter()
                .map(|row| (row.invoke_id, row.occurred_at)),
        );
    }
    Ok(terminal_keys)
}

pub(crate) fn overlay_runtime_records_for_current_page(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    runtime_records: Vec<ApiInvocation>,
    db_runtime_keys: &HashSet<(String, String)>,
    db_terminal_keys: &HashSet<(String, String)>,
    db_records_are_prefix: bool,
    mut records: Vec<ApiInvocation>,
    total: i64,
    endpoint: &'static str,
) -> (Vec<ApiInvocation>, i64) {
    if runtime_records.is_empty() {
        return (records, total);
    }
    let runtime_by_key = runtime_records
        .into_iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut runtime_overlay_row_count = 0_usize;
    let mut stale_db_runtime_row_count = 0_usize;
    records = records
        .into_iter()
        .filter_map(|mut record| {
            let key = (record.invoke_id.clone(), record.occurred_at.clone());
            let Some(runtime_record) = runtime_by_key.get(&key) else {
                return Some(record);
            };
            match normalized_runtime_text(record.status.as_deref()).as_str() {
                "running" | "pending" => {
                    if runtime_record_matches_filters(
                        runtime_record,
                        &request.filters,
                        source_scope,
                    ) {
                        record = runtime_record.clone();
                        runtime_overlay_row_count += 1;
                        Some(record)
                    } else {
                        stale_db_runtime_row_count += 1;
                        None
                    }
                }
                _ => Some(record),
            }
        })
        .collect();
    let stale_db_runtime_total_count = db_runtime_keys
        .iter()
        .filter(|key| {
            runtime_by_key.get(*key).is_some_and(|record| {
                !runtime_record_matches_filters(record, &request.filters, source_scope)
            })
        })
        .count();
    let runtime_new_records = runtime_by_key
        .iter()
        .filter(|(key, record)| {
            !db_runtime_keys.contains(*key)
                && !db_terminal_keys.contains(*key)
                && runtime_record_matches_filters(record, &request.filters, source_scope)
        })
        .map(|(_, record)| record.clone())
        .collect::<Vec<_>>();
    let runtime_new_row_count = runtime_new_records.len();
    let effective_stale_db_runtime_total_count = if db_runtime_keys.is_empty() {
        stale_db_runtime_row_count
    } else {
        stale_db_runtime_total_count
    };
    runtime_overlay_row_count += runtime_new_row_count;
    records.extend(runtime_new_records);
    records.sort_by(|left, right| {
        compare_runtime_invocation_records(left, right, request.sort_by, request.sort_order)
    });
    if db_records_are_prefix {
        let offset = (request.page - 1).saturating_mul(request.page_size) as usize;
        records = records
            .into_iter()
            .skip(offset)
            .take(request.page_size as usize)
            .collect();
    } else {
        records.truncate(request.page_size as usize);
    }
    if runtime_overlay_row_count > 0 {
        debug!(
            endpoint,
            runtime_overlay_row_count,
            stale_db_runtime_row_count,
            stale_db_runtime_total_count = effective_stale_db_runtime_total_count,
            "overlayed memory runtime invocation records into current response"
        );
    }
    (
        records,
        total.saturating_sub(effective_stale_db_runtime_total_count as i64)
            + runtime_new_row_count as i64,
    )
}

pub(crate) fn runtime_overlay_total_delta(
    request: &InvocationListRequest,
    source_scope: InvocationSourceScope,
    runtime_records: &[ApiInvocation],
    db_runtime_keys: &HashSet<(String, String)>,
    db_terminal_keys: &HashSet<(String, String)>,
) -> Result<(RuntimeSummaryOverlayDelta, usize, usize), ApiError> {
    if runtime_records.is_empty() {
        return Ok((RuntimeSummaryOverlayDelta::default(), 0, 0));
    }
    let runtime_by_key = runtime_records
        .iter()
        .map(|record| {
            (
                (record.invoke_id.clone(), record.occurred_at.clone()),
                record,
            )
        })
        .collect::<HashMap<_, _>>();
    let stale_db_runtime_count = db_runtime_keys
        .iter()
        .filter(|key| {
            runtime_by_key.get(*key).is_some_and(|record| {
                !runtime_record_matches_filters(record, &request.filters, source_scope)
            })
        })
        .count();
    let stale_db_runtime_count = i64::try_from(stale_db_runtime_count)
        .map_err(|_| summary_live_numeric_overflow("stale runtime count"))?;
    let mut delta = RuntimeSummaryOverlayDelta {
        total_count: -stale_db_runtime_count,
        ..RuntimeSummaryOverlayDelta::default()
    };
    let mut runtime_new_count = 0_usize;
    for (key, record) in &runtime_by_key {
        if db_terminal_keys.contains(key)
            || !runtime_record_matches_filters(record, &request.filters, source_scope)
        {
            continue;
        }
        let has_db_runtime_row = db_runtime_keys.contains(key);
        if !has_db_runtime_row {
            checked_summary_live_add_i64(&mut delta.total_count, 1, "runtime total count")?;
            runtime_new_count = runtime_new_count
                .checked_add(1)
                .ok_or_else(|| summary_live_numeric_overflow("runtime new count"))?;
        }
        if runtime_record_is_in_flight(record) {
            continue;
        }
        delta.add_terminal_record(record)?;
    }
    Ok((delta, runtime_new_count, stale_db_runtime_count as usize))
}

#[derive(Debug, Default)]
pub(crate) struct RuntimeSummaryOverlayDelta {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) service_failure_count: i64,
    pub(crate) client_failure_count: i64,
    pub(crate) client_abort_count: i64,
    pub(crate) max_total_tokens: Option<i64>,
    pub(crate) max_total_ms: Option<f64>,
}

impl RuntimeSummaryOverlayDelta {
    pub(crate) fn add_terminal_record(&mut self, record: &ApiInvocation) -> Result<(), ApiError> {
        for (field, value) in [
            ("runtime total tokens", record.total_tokens),
            ("runtime input tokens", record.input_tokens),
            ("runtime cache input tokens", record.cache_input_tokens),
            ("runtime output tokens", record.output_tokens),
            (
                "runtime cache write tokens",
                resolve_invocation_cache_write_tokens(record),
            ),
        ] {
            if value.is_some_and(|value| value < 0) {
                return Err(summary_live_numeric_overflow(field));
            }
        }
        let cost = record.cost.unwrap_or_default();
        if !cost.is_finite() || cost < 0.0 {
            return Err(summary_live_numeric_overflow("runtime total cost"));
        }
        checked_summary_live_add_i64(
            &mut self.total_tokens,
            record.total_tokens.unwrap_or_default(),
            "runtime total tokens",
        )?;
        checked_summary_live_add(&mut self.total_cost, cost, "runtime total cost")?;
        checked_summary_live_add_i64(
            &mut self.cache_write_tokens,
            resolve_invocation_cache_write_tokens(record).unwrap_or_default(),
            "runtime cache write tokens",
        )?;
        checked_summary_live_add_i64(
            &mut self.cache_input_tokens,
            record.cache_input_tokens.unwrap_or_default(),
            "runtime cache input tokens",
        )?;
        checked_summary_live_add_i64(
            &mut self.output_tokens,
            record.output_tokens.unwrap_or_default(),
            "runtime output tokens",
        )?;
        let failure_class = normalized_runtime_text(record.failure_class.as_deref());
        if failure_class == "none" && runtime_record_is_success_for_summary(record) {
            checked_summary_live_add_i64(&mut self.success_count, 1, "runtime success count")?;
        }
        match failure_class.as_str() {
            "service_failure" => {
                checked_summary_live_add_i64(&mut self.failure_count, 1, "runtime failure count")?;
                checked_summary_live_add_i64(
                    &mut self.service_failure_count,
                    1,
                    "runtime service failure count",
                )?;
            }
            "client_failure" => {
                checked_summary_live_add_i64(&mut self.failure_count, 1, "runtime failure count")?;
                checked_summary_live_add_i64(
                    &mut self.client_failure_count,
                    1,
                    "runtime client failure count",
                )?;
            }
            "client_abort" => {
                checked_summary_live_add_i64(&mut self.failure_count, 1, "runtime failure count")?;
                checked_summary_live_add_i64(
                    &mut self.client_abort_count,
                    1,
                    "runtime client abort count",
                )?;
            }
            _ => {}
        }

        if let Some(total_tokens) = record.total_tokens.filter(|value| *value >= 0) {
            self.max_total_tokens = match self.max_total_tokens {
                Some(current) => Some(current.max(total_tokens)),
                None => Some(total_tokens),
            };
        }

        if let Some(total_ms) = record
            .t_total_ms
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            self.max_total_ms = match self.max_total_ms {
                Some(current) => Some(current.max(total_ms)),
                None => Some(total_ms),
            };
        }
        Ok(())
    }
}

pub(crate) fn runtime_record_is_success_for_summary(record: &ApiInvocation) -> bool {
    let status = normalized_runtime_text(record.status.as_deref());
    status == "success"
        || status == "succeeded"
        || status == "completed"
        || status == "ok"
        || status == INVOCATION_STATUS_WARNING_SUCCESS
        || (status == "http_200"
            && normalized_runtime_text(record.error_message.as_deref()).is_empty())
}

pub(crate) async fn query_invocation_network_summary(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationNetworkSummary> {
    let mut connection = pool.acquire().await?;
    query_invocation_network_summary_on_connection(
        &mut connection,
        filters,
        source_scope,
        snapshot_id,
    )
    .await
}

pub(crate) async fn query_invocation_network_summary_on_connection(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<InvocationNetworkSummary> {
    #[derive(Debug, FromRow)]
    struct ValueRow {
        value: Option<f64>,
    }

    let ttfb_nonnegative_sql = sqlite_nonnegative_timing_sql("t_upstream_ttfb_ms");
    let final_first_token_sql =
        final_pool_invocation_timing_sql("codex_invocations", "first_token_ms");
    let final_stream_sql =
        final_pool_invocation_timing_sql("codex_invocations", "t_upstream_stream_ms");
    let total_nonnegative_sql = sqlite_nonnegative_timing_sql("t_total_ms");
    let mut agg_query = QueryBuilder::new(format!(
        "SELECT \
         AVG(CASE WHEN {ttfb_nonnegative_sql} THEN t_upstream_ttfb_ms END) AS avg_ttfb_ms, \
         COALESCE(SUM(CASE WHEN {ttfb_nonnegative_sql} THEN 1 ELSE 0 END), 0) AS ttfb_count, \
         AVG({final_first_token_sql}) AS avg_first_token_ms, \
         COALESCE(SUM(CASE WHEN ({final_first_token_sql}) IS NOT NULL THEN 1 ELSE 0 END), 0) AS first_token_count, \
         AVG({final_stream_sql}) AS avg_response_duration_ms, \
         COALESCE(SUM(CASE WHEN ({final_stream_sql}) IS NOT NULL THEN 1 ELSE 0 END), 0) AS response_duration_count, \
         AVG(CASE WHEN {total_nonnegative_sql} THEN t_total_ms END) AS avg_total_ms, \
         COALESCE(SUM(CASE WHEN {total_nonnegative_sql} THEN 1 ELSE 0 END), 0) AS total_count, \
         MAX(CASE WHEN {total_nonnegative_sql} THEN t_total_ms END) AS max_total_ms \
         FROM codex_invocations WHERE 1 = 1"
    ));
    apply_invocation_records_filters(
        &mut agg_query,
        filters,
        source_scope,
        Some(SnapshotConstraint::UpTo(snapshot_id)),
    );
    let agg = agg_query
        .build_query_as::<InvocationNetworkAggRow>()
        .fetch_one(&mut *connection)
        .await?;

    async fn query_sorted_value(
        connection: &mut SqliteConnection,
        filters: &InvocationRecordsFilters,
        source_scope: InvocationSourceScope,
        snapshot_id: i64,
        column: &'static str,
        offset: i64,
    ) -> Result<Option<f64>> {
        let timing_value_sql = match column {
            "first_token_ms" | "t_upstream_stream_ms" => {
                final_pool_invocation_timing_sql("codex_invocations", column)
            }
            _ => column.to_string(),
        };
        let mut query = QueryBuilder::new("SELECT ");
        query
            .push(timing_value_sql.as_str())
            .push(" AS value FROM codex_invocations WHERE 1 = 1");
        apply_invocation_records_filters(
            &mut query,
            filters,
            source_scope,
            Some(SnapshotConstraint::UpTo(snapshot_id)),
        );
        let timing_sql = match column {
            "first_token_ms" | "t_upstream_stream_ms" => {
                format!("({timing_value_sql}) IS NOT NULL")
            }
            _ => sqlite_nonnegative_timing_sql(column),
        };
        query.push(" AND ").push(timing_sql.as_str());
        query.push(" ORDER BY value ASC");
        query.push(" LIMIT 1 OFFSET ").push_bind(offset.max(0));

        Ok(query
            .build_query_as::<ValueRow>()
            .fetch_optional(connection)
            .await?
            .and_then(|row| row.value))
    }

    fn resolve_p95_offsets(count: i64) -> Option<(i64, i64, f64)> {
        if count <= 0 {
            return None;
        }
        if count == 1 {
            return Some((0, 0, 0.0));
        }

        let rank = 0.95_f64 * (count.saturating_sub(1) as f64);
        let lower = rank.floor() as i64;
        let upper = rank.ceil() as i64;
        let weight = rank - lower as f64;
        Some((lower, upper, weight))
    }

    async fn query_p95(
        connection: &mut SqliteConnection,
        filters: &InvocationRecordsFilters,
        source_scope: InvocationSourceScope,
        snapshot_id: i64,
        column: &'static str,
        count: i64,
    ) -> Result<Option<f64>> {
        let Some((lower, upper, weight)) = resolve_p95_offsets(count) else {
            return Ok(None);
        };

        let Some(lower_value) = query_sorted_value(
            connection,
            filters,
            source_scope,
            snapshot_id,
            column,
            lower,
        )
        .await?
        else {
            return Ok(None);
        };

        if lower == upper {
            return Ok(Some(lower_value));
        }

        let Some(upper_value) = query_sorted_value(
            connection,
            filters,
            source_scope,
            snapshot_id,
            column,
            upper,
        )
        .await?
        else {
            return Ok(None);
        };

        Ok(Some(lower_value + (upper_value - lower_value) * weight))
    }

    Ok(InvocationNetworkSummary {
        avg_ttfb_ms: agg.avg_ttfb_ms,
        p95_ttfb_ms: query_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "t_upstream_ttfb_ms",
            agg.ttfb_count,
        )
        .await?,
        avg_first_token_ms: agg.avg_first_token_ms,
        p95_first_token_ms: query_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "first_token_ms",
            agg.first_token_count,
        )
        .await?,
        avg_response_duration_ms: agg.avg_response_duration_ms,
        p95_response_duration_ms: query_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "t_upstream_stream_ms",
            agg.response_duration_count,
        )
        .await?,
        avg_total_ms: agg.avg_total_ms,
        p95_total_ms: query_p95(
            connection,
            filters,
            source_scope,
            snapshot_id,
            "t_total_ms",
            agg.total_count,
        )
        .await?,
        max_total_ms: agg.max_total_ms,
    })
}

pub(crate) async fn query_invocation_new_records_count(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<i64> {
    let mut connection = pool.acquire().await?;
    query_invocation_new_records_count_on_connection(
        &mut connection,
        filters,
        source_scope,
        snapshot_id,
    )
    .await
}

pub(crate) async fn query_invocation_new_records_count_on_connection(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot_id: i64,
) -> Result<i64> {
    #[derive(Debug, FromRow)]
    struct NewCountRow {
        total: i64,
    }

    let mut new_count_query =
        QueryBuilder::new("SELECT COUNT(*) AS total FROM codex_invocations WHERE 1 = 1");
    apply_invocation_records_filters(
        &mut new_count_query,
        filters,
        source_scope,
        Some(SnapshotConstraint::After(snapshot_id)),
    );

    Ok(new_count_query
        .build_query_as::<NewCountRow>()
        .fetch_one(connection)
        .await?
        .total)
}
