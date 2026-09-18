include!("request_entry/part_01.rs");
include!("request_entry/part_02.rs");
include!("request_entry/part_03.rs");
#[derive(Debug)]
pub(crate) struct PoolUpstreamError {
    pub(crate) account: Option<PoolResolvedAccount>,
    pub(crate) status: StatusCode,
    pub(crate) message: String,
    pub(crate) canonical_error_message: Option<String>,
    pub(crate) failure_kind: &'static str,
    pub(crate) blocked_binding: Option<BlockedBindingDiagnostic>,
    pub(crate) connect_latency_ms: f64,
    pub(crate) upstream_error_code: Option<String>,
    pub(crate) upstream_error_message: Option<String>,
    pub(crate) downstream_error_message: Option<String>,
    pub(crate) upstream_request_id: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) oauth_responses_debug: Option<oauth_bridge::OauthResponsesDebugInfo>,
    pub(crate) attempt_summary: PoolAttemptSummary,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) codex_imagegen_rewrite: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PoolAttemptSummary {
    pub(crate) pool_attempt_count: usize,
    pub(crate) pool_distinct_account_count: usize,
    pub(crate) pool_attempt_terminal_reason: Option<String>,
    pub(crate) pool_routing_no_candidate_audit: Option<PoolRoutingNoCandidateAudit>,
}

pub(crate) fn pool_attempt_summary(
    pool_attempt_count: usize,
    pool_distinct_account_count: usize,
    pool_attempt_terminal_reason: Option<String>,
) -> PoolAttemptSummary {
    PoolAttemptSummary {
        pool_attempt_count,
        pool_distinct_account_count,
        pool_attempt_terminal_reason,
        pool_routing_no_candidate_audit: None,
    }
}

pub(crate) fn pool_upstream_error_is_rate_limited(err: &PoolUpstreamError) -> bool {
    err.status == StatusCode::TOO_MANY_REQUESTS
        || matches!(
            err.failure_kind,
            FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED
                | PROXY_FAILURE_POOL_ALL_ACCOUNTS_RATE_LIMITED
        )
}

pub(crate) fn build_pool_rate_limited_error(
    attempt_count: usize,
    distinct_account_count: usize,
    failure_kind: &'static str,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::TOO_MANY_REQUESTS,
        message: POOL_ALL_ACCOUNTS_RATE_LIMITED_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn build_pool_no_available_account_error(
    attempt_count: usize,
    distinct_account_count: usize,
    _retry_after_secs: u64,
    no_candidate_audit: Option<PoolRoutingNoCandidateAudit>,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_NO_AVAILABLE_ACCOUNT_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: PoolAttemptSummary {
            pool_routing_no_candidate_audit: no_candidate_audit,
            ..pool_attempt_summary(
                attempt_count,
                distinct_account_count,
                Some(PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT.to_string()),
            )
        },
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) const PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE: &str =
    "encrypted_session_owner_unavailable";
pub(crate) const ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE: &str = "encrypted session owner unavailable; automatic routing cannot move this encrypted conversation";

pub(crate) fn build_encrypted_session_owner_unavailable_error(
    account: Option<PoolResolvedAccount>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE.to_string()),
        upstream_error_message: Some(ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE.to_string()),
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_ENCRYPTED_SESSION_OWNER_UNAVAILABLE.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn build_pool_assigned_account_blocked_error(
    account: PoolResolvedAccount,
    message: String,
    failure_kind: &'static str,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    build_pool_assigned_binding_blocked_error(
        Some(account),
        message,
        failure_kind,
        None,
        attempt_count,
        distinct_account_count,
    )
}

pub(crate) fn build_pool_assigned_binding_blocked_error(
    account: Option<PoolResolvedAccount>,
    message: String,
    failure_kind: &'static str,
    blocked_binding: Option<BlockedBindingDiagnostic>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message,
        canonical_error_message: None,
        failure_kind,
        blocked_binding,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(failure_kind.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn retry_after_secs_for_proxy_error(status: StatusCode, message: &str) -> Option<u64> {
    if status != StatusCode::SERVICE_UNAVAILABLE {
        return None;
    }
    if message == POOL_NO_AVAILABLE_ACCOUNT_MESSAGE
        || message == ENCRYPTED_SESSION_OWNER_UNAVAILABLE_MESSAGE
    {
        return Some(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS);
    }
    None
}

pub(crate) fn build_pool_degraded_only_error(
    attempt_count: usize,
    distinct_account_count: usize,
) -> PoolUpstreamError {
    PoolUpstreamError {
        codex_imagegen_rewrite: None,
        account: None,
        status: StatusCode::SERVICE_UNAVAILABLE,
        message: POOL_ALL_ACCOUNTS_DEGRADED_MESSAGE.to_string(),
        canonical_error_message: None,
        failure_kind: PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED,
        blocked_binding: None,
        connect_latency_ms: 0.0,
        upstream_error_code: None,
        upstream_error_message: None,
        downstream_error_message: None,
        upstream_request_id: None,
        proxy_binding_key_snapshot: None,
        oauth_responses_debug: None,
        attempt_summary: pool_attempt_summary(
            attempt_count,
            distinct_account_count,
            Some(PROXY_FAILURE_POOL_ALL_ACCOUNTS_DEGRADED.to_string()),
        ),
        requested_service_tier: None,
        request_body_for_capture: None,
    }
}

pub(crate) fn pool_upstream_error_preserves_existing_sticky_owner(
    err: Option<&PoolUpstreamError>,
) -> bool {
    err.and_then(|value| value.account.as_ref())
        .is_some_and(|account| account.routing_source == PoolRoutingSelectionSource::StickyReuse)
        && matches!(
            err.map(|value| value.failure_kind),
            Some(
                FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429
                    | FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX
                    | PROXY_FAILURE_FAILED_CONTACT_UPSTREAM
                    | PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT
                    | PROXY_FAILURE_UPSTREAM_STREAM_ERROR
                    | PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED
            )
        )
}

pub(crate) fn pool_upstream_error_has_concrete_account_context(
    err: Option<&PoolUpstreamError>,
) -> bool {
    err.and_then(|value| value.account.as_ref()).is_some()
}

pub(crate) fn sticky_owner_terminal_error_preservation_is_active(
    preserve_sticky_owner_terminal_error: bool,
    err: Option<&PoolUpstreamError>,
) -> bool {
    preserve_sticky_owner_terminal_error && pool_upstream_error_has_concrete_account_context(err)
}

pub(crate) fn take_sticky_owner_terminal_error(
    preserve_sticky_owner_terminal_error: bool,
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    if !sticky_owner_terminal_error_preservation_is_active(
        preserve_sticky_owner_terminal_error,
        last_error.as_ref(),
    ) {
        return None;
    }
    let mut err = last_error.take()?;
    if err.status.is_success() {
        err.status = StatusCode::SERVICE_UNAVAILABLE;
    }
    err.attempt_summary = pool_attempt_summary(
        attempt_count,
        distinct_account_count,
        Some(err.failure_kind.to_string()),
    );
    Some(err)
}

pub(crate) async fn take_and_record_sticky_owner_terminal_error(
    state: &AppState,
    trace_context: Option<&PoolUpstreamAttemptTraceContext>,
    preserve_sticky_owner_terminal_error: bool,
    last_error: &mut Option<PoolUpstreamError>,
    attempt_count: usize,
    distinct_account_count: usize,
) -> Option<PoolUpstreamError> {
    let err = take_sticky_owner_terminal_error(
        preserve_sticky_owner_terminal_error,
        last_error,
        attempt_count,
        distinct_account_count,
    )?;
    if let Some(trace) = trace_context
        && let Err(record_err) = insert_and_broadcast_pool_upstream_terminal_attempt(
            state,
            trace,
            &err,
            (attempt_count + 1) as i64,
            distinct_account_count as i64,
            err.failure_kind,
        )
        .await
    {
        warn!(
            invoke_id = trace.invoke_id,
            error = %record_err,
            "failed to persist preserved sticky-owner terminal attempt"
        );
    }
    Some(err)
}

pub(crate) fn store_pool_failover_error(
    last_error: &mut Option<PoolUpstreamError>,
    preserve_sticky_owner_terminal_error: &mut bool,
    mut err: PoolUpstreamError,
    codex_imagegen_rewrite: Option<&Value>,
) {
    if err.codex_imagegen_rewrite.is_none() {
        err.codex_imagegen_rewrite = codex_imagegen_rewrite.cloned();
    }
    *preserve_sticky_owner_terminal_error |=
        pool_upstream_error_preserves_existing_sticky_owner(Some(&err));
    *last_error = Some(err);
}

#[derive(Debug, Clone)]
pub(crate) struct PendingPoolAttemptRecord {
    pub(crate) attempt_id: Option<i64>,
    pub(crate) attempt_public_id: Option<String>,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) routing_source: Option<String>,
    pub(crate) routing_selection_audit_json: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) upstream_base_url_host: Option<String>,
    pub(crate) group_name_snapshot: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) upstream_account_id: i64,
    pub(crate) upstream_route_key: String,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) started_at: String,
    pub(crate) connect_latency_ms: f64,
    pub(crate) first_byte_latency_ms: f64,
    pub(crate) compact_support_status: Option<String>,
    pub(crate) compact_support_reason: Option<String>,
    pub(crate) upstream_request_compression_algorithm: Option<String>,
    pub(crate) upstream_request_compression_mode: Option<String>,
    pub(crate) upstream_request_logical_body_bytes: Option<i64>,
    pub(crate) upstream_request_transmitted_body_bytes: Option<i64>,
    pub(crate) upstream_request_header_bytes_approx: Option<i64>,
    pub(crate) upstream_response_body_bytes: Option<i64>,
    pub(crate) upstream_response_header_bytes_approx: Option<i64>,
    pub(crate) response_raw_path: Option<String>,
    pub(crate) response_raw_codec: Option<String>,
    pub(crate) response_raw_size: Option<i64>,
    pub(crate) response_raw_truncated: bool,
    pub(crate) response_raw_truncated_reason: Option<String>,
    pub(crate) response_content_encoding: Option<String>,
}

#[derive(Debug, Default)]
pub(crate) struct PoolFailoverProgress {
    pub(crate) excluded_account_ids: Vec<i64>,
    pub(crate) excluded_upstream_route_keys: HashSet<String>,
    pub(crate) attempt_count: usize,
    pub(crate) last_error: Option<PoolUpstreamError>,
    pub(crate) preserve_sticky_owner_terminal_error: bool,
    pub(crate) overload_required_upstream_route_key: Option<String>,
    pub(crate) timeout_route_failover_pending: bool,
    pub(crate) responses_total_timeout_started_at: Option<Instant>,
    pub(crate) no_available_wait_deadline: Option<Instant>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolUpstreamAttemptTraceContext {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) upstream_base_url_host: Option<String>,
    pub(crate) request_model: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolAttemptRuntimeSnapshotContext {
    pub(crate) capture_target: ProxyCaptureTarget,
    pub(crate) request_info: RequestCaptureInfo,
    pub(crate) hosted_image_intent: Option<ImageIntent>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) owner_auto_guard_active: bool,
    pub(crate) t_req_read_ms: f64,
    pub(crate) t_req_parse_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct InvocationRecoverySelector {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl InvocationRecoverySelector {
    pub(crate) fn new(invoke_id: impl Into<String>, occurred_at: impl Into<String>) -> Self {
        Self {
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
        }
    }
}

impl From<&PendingPoolAttemptRecord> for InvocationRecoverySelector {
    fn from(value: &PendingPoolAttemptRecord) -> Self {
        Self::new(value.invoke_id.clone(), value.occurred_at.clone())
    }
}

#[derive(Debug, Clone, FromRow)]
#[allow(dead_code)]
pub(crate) struct RecoveredPoolAttemptRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct RecoveredInvocationRow {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PoolOrphanRecoveryOutcome {
    pub(crate) recovered_attempts: usize,
    pub(crate) recovered_invocations: usize,
}

pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING: &str = "pending";
pub(crate) struct CompactSupportObservation {
    pub(crate) status: &'static str,
    pub(crate) reason: Option<String>,
}
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS: &str = "success";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE: &str = "http_failure";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE: &str = "transport_failure";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL: &str =
    "budget_exhausted_final";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_CONNECTING: &str = "connecting";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_SENDING_REQUEST: &str = "sending_request";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_WAITING_FIRST_BYTE: &str =
    "waiting_first_byte";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_STREAMING_RESPONSE: &str =
    "streaming_response";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED: &str = "completed";
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED: &str = "failed";
pub(crate) const POOL_VIA_INVOKE_ID_PREFIX: &str = "pool-via-";
pub(crate) const POOL_EARLY_PHASE_ORPHAN_RECOVERY_GRACE: Duration = Duration::from_secs(30);
pub(crate) const POOL_ATTEMPT_RECOVERY_SELECTOR_BATCH_SIZE: usize = 400;
pub(crate) const PROXY_INVOCATION_RECOVERY_SELECTOR_BATCH_SIZE: usize = 400;

pub(crate) struct PoolEarlyPhaseOrphanCleanupGuard {
    state: Arc<AppState>,
    pending_attempt_record: PendingPoolAttemptRecord,
    pub(crate) first_byte_observed: bool,
    pub(crate) terminal_outcome_observed: bool,
    pub(crate) armed: bool,
}

pub(crate) struct PoolViaRuntimeSnapshotCleanupGuard {
    state: Arc<AppState>,
    invoke_id: String,
}

impl PoolViaRuntimeSnapshotCleanupGuard {
    pub(crate) fn new(state: Arc<AppState>, proxy_request_id: u64) -> Self {
        Self {
            state,
            invoke_id: format!("{POOL_VIA_INVOKE_ID_PREFIX}{proxy_request_id}"),
        }
    }
}

impl Drop for PoolViaRuntimeSnapshotCleanupGuard {
    fn drop(&mut self) {
        let removed_records = self
            .state
            .proxy_runtime_invocations
            .remove_non_terminal_by_invoke_id(&self.invoke_id);
        if !removed_records.is_empty() {
            for record in &removed_records {
                self.state
                    .subscription_hub
                    .publish_runtime_mutation(RuntimeMutation::invocation(
                        record,
                        RuntimeMutationKind::RuntimeRemoved,
                    ));
            }
            schedule_dashboard_activity_live_snapshot(self.state.as_ref());
        }
        debug!(
            invoke_id = %self.invoke_id,
            removed_count = removed_records.len(),
            "request-scoped via-pool runtime snapshots cleaned up"
        );
    }
}

impl std::fmt::Debug for PoolEarlyPhaseOrphanCleanupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolEarlyPhaseOrphanCleanupGuard")
            .field("pending_attempt_record", &self.pending_attempt_record)
            .field("armed", &self.armed)
            .finish()
    }
}

impl PoolEarlyPhaseOrphanCleanupGuard {
    pub(crate) fn new(
        state: Arc<AppState>,
        pending_attempt_record: PendingPoolAttemptRecord,
    ) -> Self {
        Self {
            state,
            pending_attempt_record,
            first_byte_observed: false,
            terminal_outcome_observed: false,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }

    pub(crate) fn mark_first_byte_observed(&mut self, first_byte_latency_ms: f64) {
        self.first_byte_observed = true;
        self.pending_attempt_record.first_byte_latency_ms = self
            .pending_attempt_record
            .first_byte_latency_ms
            .max(first_byte_latency_ms);
    }

    pub(crate) fn mark_terminal_outcome_observed(&mut self) {
        self.terminal_outcome_observed = true;
    }
}

impl Drop for PoolEarlyPhaseOrphanCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let state = self.state.clone();
        let pending_attempt_record = self.pending_attempt_record.clone();
        let first_byte_observed = self.first_byte_observed;
        let terminal_outcome_observed = self.terminal_outcome_observed;
        tokio::spawn(async move {
            if let Err(err) = recover_guard_dropped_pool_early_phase_orphan(
                state.as_ref(),
                pending_attempt_record,
                first_byte_observed,
                terminal_outcome_observed,
            )
            .await
            {
                warn!(error = %err, "failed to recover dropped pool early-phase orphan");
            }
        });
    }
}

pub(crate) struct PoolInvocationCleanupGuard {
    state: Arc<AppState>,
    selector: InvocationRecoverySelector,
    recovery_trigger: &'static str,
    armed: bool,
}

impl std::fmt::Debug for PoolInvocationCleanupGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolInvocationCleanupGuard")
            .field("selector", &self.selector)
            .field("recovery_trigger", &self.recovery_trigger)
            .field("armed", &self.armed)
            .finish()
    }
}

impl PoolInvocationCleanupGuard {
    pub(crate) fn new(
        state: Arc<AppState>,
        selector: InvocationRecoverySelector,
        recovery_trigger: &'static str,
    ) -> Self {
        Self {
            state,
            selector,
            recovery_trigger,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PoolInvocationCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }

        let state = self.state.clone();
        let selector = self.selector.clone();
        let recovery_trigger = self.recovery_trigger;
        tokio::spawn(async move {
            if let Err(err) = recover_guard_dropped_pool_invocation_orphan(
                state.as_ref(),
                selector,
                recovery_trigger,
            )
            .await
            {
                warn!(error = %err, recovery_trigger, "failed to recover dropped pool invocation orphan");
            }
        });
    }
}

pub(crate) struct PoolLiveAttemptActivityLease {
    live_attempt_ids: Arc<std::sync::Mutex<HashSet<i64>>>,
    attempt_id: i64,
}

impl std::fmt::Debug for PoolLiveAttemptActivityLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PoolLiveAttemptActivityLease")
            .field("attempt_id", &self.attempt_id)
            .finish()
    }
}

impl PoolLiveAttemptActivityLease {
    pub(crate) fn new(state: Arc<AppState>, attempt_id: i64) -> Self {
        {
            let mut live_attempt_ids = state
                .pool_live_attempt_ids
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            live_attempt_ids.insert(attempt_id);
        }
        Self {
            live_attempt_ids: state.pool_live_attempt_ids.clone(),
            attempt_id,
        }
    }
}

impl Drop for PoolLiveAttemptActivityLease {
    fn drop(&mut self) {
        let mut live_attempt_ids = self
            .live_attempt_ids
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        live_attempt_ids.remove(&self.attempt_id);
    }
}

pub(crate) fn disarm_pool_early_phase_cleanup_guard(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
) {
    if let Some(guard) = guard.as_mut() {
        guard.disarm();
    }
}

pub(crate) fn complete_deferred_pool_early_phase_cleanup_guard(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
) {
    if let Some(guard) = guard.as_mut() {
        guard.mark_terminal_outcome_observed();
    }
    disarm_pool_early_phase_cleanup_guard(guard);
}

pub(crate) fn finalize_deferred_pool_early_phase_cleanup_guard_after_terminal_invocation(
    guard: &mut Option<PoolEarlyPhaseOrphanCleanupGuard>,
    terminal_invocation_persisted: bool,
) {
    if !terminal_invocation_persisted || guard.is_none() {
        return;
    }
    complete_deferred_pool_early_phase_cleanup_guard(guard);
}

pub(crate) fn disarm_pool_invocation_cleanup_guard(guard: &mut Option<PoolInvocationCleanupGuard>) {
    if let Some(guard) = guard.as_mut() {
        guard.disarm();
    }
}
pub(crate) const POOL_UPSTREAM_MAX_DISTINCT_ACCOUNTS: usize = 3;
pub(crate) const POOL_UPSTREAM_RESPONSES_MAX_TIMEOUT_ROUTE_KEYS: usize = 3;
pub(crate) const REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES: usize = 64 * 1024;
pub(crate) const ENV_PROXY_REQUEST_SEMANTIC_PIPELINE_MODE: &str =
    "PROXY_REQUEST_SEMANTIC_PIPELINE_MODE";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum RequestSemanticPipelineMode {
    Projection,
    Legacy,
}

impl RequestSemanticPipelineMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Projection => "projection",
            Self::Legacy => "legacy",
        }
    }
}

pub(crate) fn request_semantic_pipeline_mode() -> RequestSemanticPipelineMode {
    match std::env::var(ENV_PROXY_REQUEST_SEMANTIC_PIPELINE_MODE) {
        Ok(value) if value.trim().eq_ignore_ascii_case("legacy") => {
            RequestSemanticPipelineMode::Legacy
        }
        _ => RequestSemanticPipelineMode::Projection,
    }
}

#[derive(Debug)]
pub(crate) struct PoolReplayTempFile {
    pub(crate) path: PathBuf,
}

impl Drop for PoolReplayTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodySnapshot {
    Empty,
    Memory(Bytes),
    File {
        temp_file: Arc<PoolReplayTempFile>,
        size: usize,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodyStatus {
    Reading,
    Complete(PoolReplayBodySnapshot),
    ReadError(RequestBodyReadError),
    InternalError(String),
    Incomplete,
}

/// Immutable request semantics derived from the single replay snapshot.
///
/// The snapshot remains the source of truth for forwarding and raw capture. The
/// projection only owns bounded business metadata and an optional rewritten
/// snapshot, so large request bodies never need to be copied into a `Vec`.
#[derive(Debug, Clone)]
pub(crate) struct RequestSemanticProjection {
    pub(crate) snapshot: PoolReplayBodySnapshot,
    pub(crate) request_info: RequestCaptureInfo,
    pub(crate) hosted_image_intent: ImageIntent,
    pub(crate) upstream_snapshot: PoolReplayBodySnapshot,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) body_rewritten: bool,
    pub(crate) parse_elapsed_ms: u64,
    pub(crate) materialization_bytes: usize,
    pub(crate) buffer_bytes: usize,
    pub(crate) json_parse_count: u8,
    pub(crate) whole_body_materialization_count: u8,
    pub(crate) peak_business_buffer_bytes: usize,
    pub(crate) fallback_reason: Option<&'static str>,
}

pub(crate) struct PoolReplayBodyBuffer {
    proxy_request_id: u64,
    len: usize,
    memory: Vec<u8>,
    file: Option<(Arc<PoolReplayTempFile>, tokio::fs::File)>,
}

pub(crate) fn proxy_forward_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
) -> bool {
    !stream_error && status != StatusCode::TOO_MANY_REQUESTS && !status.is_server_error()
}

pub(crate) fn proxy_capture_response_status_is_success(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> bool {
    !logical_stream_failure && proxy_forward_response_status_is_success(status, stream_error)
}

pub(crate) fn proxy_capture_is_pure_downstream_close(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    downstream_closed: bool,
) -> bool {
    downstream_closed && status.is_success() && !stream_error && !logical_stream_failure
}

pub(crate) fn proxy_capture_invocation_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    pure_downstream_closed: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    } else if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else if pure_downstream_closed {
        Some(PROXY_STREAM_TERMINAL_DOWNSTREAM_CLOSED)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}
#[derive(Debug, Clone)]
pub(crate) struct PoolRequestBodyPreparationError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl PoolRequestBodyPreparationError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRequestBodyCompressionMode {
    Identity,
    Passthrough,
    Recompressed,
}

impl PoolRequestBodyCompressionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Passthrough => "passthrough",
            Self::Recompressed => "recompressed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestBodyContentEncoding {
    Identity,
    Gzip,
    Deflate { zlib_wrapper: bool },
    Zstd,
}

impl RequestBodyContentEncoding {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Deflate { .. } => "deflate",
            Self::Zstd => "zstd",
        }
    }

    pub(crate) fn header_value(self) -> Option<&'static str> {
        match self {
            Self::Identity => None,
            Self::Gzip => Some("gzip"),
            Self::Deflate { .. } => Some("deflate"),
            Self::Zstd => Some("zstd"),
        }
    }

    pub(crate) fn algorithm(self) -> RequestCompressionAlgorithm {
        match self {
            Self::Identity => RequestCompressionAlgorithm::Identity,
            Self::Gzip => RequestCompressionAlgorithm::Gzip,
            Self::Deflate { .. } => RequestCompressionAlgorithm::Deflate,
            Self::Zstd => RequestCompressionAlgorithm::Zstd,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedPoolUpstreamRequestBody {
    pub(crate) body: Body,
    pub(crate) content_length: Option<usize>,
    pub(crate) content_encoding: RequestBodyContentEncoding,
    pub(crate) compression_mode: PoolRequestBodyCompressionMode,
    pub(crate) byte_observation: PreparedPoolUpstreamRequestBodyObservation,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObservedByteCounter {
    inner: Arc<AtomicU64>,
}

impl ObservedByteCounter {
    pub(crate) fn add(&self, bytes: usize) {
        let Ok(bytes) = u64::try_from(bytes) else {
            return;
        };
        self.inner.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn load(&self) -> usize {
        usize::try_from(self.inner.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ObservedBodyBytes {
    Fixed(usize),
    Counter(ObservedByteCounter),
}

impl ObservedBodyBytes {
    pub(crate) fn load(&self) -> usize {
        match self {
            Self::Fixed(bytes) => *bytes,
            Self::Counter(counter) => counter.load(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPoolUpstreamRequestBodyObservation {
    pub(crate) logical_body_bytes: ObservedBodyBytes,
    pub(crate) transmitted_body_bytes: ObservedByteCounter,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestCompressionObservation {
    pub(crate) algorithm: String,
    pub(crate) mode: String,
    pub(crate) logical_body_bytes: usize,
    pub(crate) transmitted_body_bytes: usize,
}

#[derive(Debug)]
struct CountingAsyncRead<R> {
    inner: R,
    counter: ObservedByteCounter,
}

impl<R> CountingAsyncRead<R> {
    fn new(inner: R, counter: ObservedByteCounter) -> Self {
        Self { inner, counter }
    }
}

impl<R> AsyncRead for CountingAsyncRead<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let filled_before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let filled_after = buf.filled().len();
            if filled_after > filled_before {
                self.counter.add(filled_after - filled_before);
            }
        }
        result
    }
}

pub(crate) fn http_visible_header_bytes_approx(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + 2 + value.as_bytes().len() + 2)
        .sum()
}

pub(crate) fn counted_http_body_from_bytes(bytes: Bytes, counter: ObservedByteCounter) -> Body {
    if bytes.is_empty() {
        return Body::from(Bytes::new());
    }
    let stream = stream::once(async move {
        counter.add(bytes.len());
        Ok::<Bytes, io::Error>(bytes)
    });
    Body::from_stream(stream)
}

fn counted_http_body_from_reader<R>(reader: R, counter: ObservedByteCounter) -> Body
where
    R: AsyncRead + Send + 'static,
{
    let stream = ReaderStream::new(reader).map(move |chunk| {
        if let Ok(bytes) = &chunk {
            counter.add(bytes.len());
        }
        chunk
    });
    Body::from_stream(stream)
}

pub(crate) fn counted_http_body_from_snapshot(
    snapshot: &PoolReplayBodySnapshot,
    counter: ObservedByteCounter,
) -> Body {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Body::from(Bytes::new()),
        PoolReplayBodySnapshot::Memory(bytes) => {
            counted_http_body_from_bytes(bytes.clone(), counter)
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let temp_file = temp_file.clone();
            let expected_size = *size;
            let stream = stream::unfold(
                Some((temp_file, expected_size, None::<tokio::fs::File>, counter)),
                |state| async move {
                    let (temp_file, remaining, file, counter) = state?;
                    if remaining == 0 {
                        return None;
                    }
                    let mut file = match file {
                        Some(file) => file,
                        None => match tokio::fs::File::open(&temp_file.path).await {
                            Ok(file) => file,
                            Err(err) => {
                                return Some((Err(io::Error::other(err.to_string())), None));
                            }
                        },
                    };
                    let mut buf = vec![0_u8; remaining.min(64 * 1024)];
                    match file.read(&mut buf).await {
                        Ok(0) => None,
                        Ok(read_len) => {
                            buf.truncate(read_len);
                            counter.add(read_len);
                            Some((
                                Ok(Bytes::from(buf)),
                                Some((temp_file, remaining - read_len, Some(file), counter)),
                            ))
                        }
                        Err(err) => Some((Err(io::Error::other(err.to_string())), None)),
                    }
                },
            );
            Body::from_stream(stream)
        }
    }
}

type BoxedPoolRequestReader = Pin<Box<dyn AsyncRead + Send>>;

fn request_compression_preset_to_async_level(
    preset: RequestCompressionLevelPreset,
) -> AsyncCompressionLevel {
    match preset {
        RequestCompressionLevelPreset::Fast => AsyncCompressionLevel::Fastest,
        RequestCompressionLevelPreset::Balanced => AsyncCompressionLevel::Default,
        RequestCompressionLevelPreset::Best => AsyncCompressionLevel::Best,
    }
}

fn resolve_request_body_content_encoding_from_prefix(
    prefix: Option<&[u8]>,
    content_encoding: Option<&str>,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() || encodings.iter().all(|encoding| encoding == "identity") {
        return Ok(RequestBodyContentEncoding::Identity);
    }
    if encodings.len() != 1 {
        return Err(PoolRequestBodyPreparationError::bad_request(format!(
            "unsupported request Content-Encoding chain: {}",
            encodings.join(", ")
        )));
    }

    match encodings[0].as_str() {
        "gzip" | "x-gzip" => Ok(RequestBodyContentEncoding::Gzip),
        "deflate" => Ok(RequestBodyContentEncoding::Deflate {
            zlib_wrapper: deflate_stream_uses_zlib_wrapper(prefix.unwrap_or_default()),
        }),
        "zstd" => Ok(RequestBodyContentEncoding::Zstd),
        other => Err(PoolRequestBodyPreparationError::bad_request(format!(
            "unsupported request Content-Encoding: {other}"
        ))),
    }
}

async fn resolve_request_body_content_encoding(
    snapshot: &PoolReplayBodySnapshot,
    content_encoding: Option<&str>,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let prefix = if parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate")
    {
        Some(snapshot.to_prefix_bytes(2).await.map_err(|err| {
            PoolRequestBodyPreparationError::bad_gateway(format!(
                "failed to inspect deflate request body header: {err}"
            ))
        })?)
    } else {
        None
    };
    resolve_request_body_content_encoding_from_prefix(
        prefix.as_ref().map(Bytes::as_ref),
        content_encoding,
    )
}

pub(crate) fn observe_request_compression_from_bytes(
    bytes: &[u8],
    content_encoding: Option<&str>,
) -> Option<RequestCompressionObservation> {
    let needs_prefix = parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate");
    let prefix = needs_prefix.then(|| &bytes[..bytes.len().min(2)]);
    let encoding =
        resolve_request_body_content_encoding_from_prefix(prefix, content_encoding).ok()?;
    let logical_body_bytes = decode_request_payload_bytes(bytes, encoding).ok()?.len();
    Some(RequestCompressionObservation {
        algorithm: encoding.algorithm().as_str().to_string(),
        mode: if matches!(encoding, RequestBodyContentEncoding::Identity) {
            PoolRequestBodyCompressionMode::Identity
                .as_str()
                .to_string()
        } else {
            PoolRequestBodyCompressionMode::Passthrough
                .as_str()
                .to_string()
        },
        logical_body_bytes,
        transmitted_body_bytes: bytes.len(),
    })
}

async fn count_decoded_request_snapshot_bytes(
    snapshot: &PoolReplayBodySnapshot,
    encoding: RequestBodyContentEncoding,
) -> Result<usize, PoolRequestBodyPreparationError> {
    if matches!(encoding, RequestBodyContentEncoding::Identity) {
        return Ok(pool_request_snapshot_body_bytes(snapshot));
    }
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(0),
        PoolReplayBodySnapshot::Memory(bytes) => {
            Ok(decode_request_payload_bytes(bytes, encoding)?.len())
        }
        PoolReplayBodySnapshot::File { .. } => {
            let raw_reader = open_pool_request_snapshot_reader(snapshot).await?;
            let mut decoded_reader = decode_pool_request_reader(raw_reader, encoding).await?;
            let mut total = 0usize;
            let mut buf = [0_u8; 64 * 1024];
            loop {
                let read_len = decoded_reader.read(&mut buf).await.map_err(|err| {
                    PoolRequestBodyPreparationError::bad_gateway(format!(
                        "failed to count decoded request body bytes: {err}"
                    ))
                })?;
                if read_len == 0 {
                    break;
                }
                total = total.saturating_add(read_len);
            }
            Ok(total)
        }
    }
}

fn decode_request_payload_bytes(
    bytes: &[u8],
    encoding: RequestBodyContentEncoding,
) -> Result<Bytes, PoolRequestBodyPreparationError> {
    match encoding {
        RequestBodyContentEncoding::Identity => Ok(Bytes::copy_from_slice(bytes)),
        RequestBodyContentEncoding::Gzip => {
            let mut decoder = GzDecoder::new(bytes);
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded).map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to decode gzip request body: {err}"
                ))
            })?;
            Ok(Bytes::from(decoded))
        }
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            let mut decoded = Vec::new();
            if zlib_wrapper {
                let mut decoder = ZlibDecoder::new(bytes);
                decoder.read_to_end(&mut decoded).map_err(|err| {
                    PoolRequestBodyPreparationError::bad_request(format!(
                        "failed to decode deflate request body: {err}"
                    ))
                })?;
            } else {
                let mut decoder = DeflateDecoder::new(bytes);
                decoder.read_to_end(&mut decoded).map_err(|err| {
                    PoolRequestBodyPreparationError::bad_request(format!(
                        "failed to decode deflate request body: {err}"
                    ))
                })?;
            }
            Ok(Bytes::from(decoded))
        }
        RequestBodyContentEncoding::Zstd => {
            zstd::decode_all(bytes).map(Bytes::from).map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to decode zstd request body: {err}"
                ))
            })
        }
    }
}

async fn open_pool_request_snapshot_reader(
    snapshot: &PoolReplayBodySnapshot,
) -> Result<BoxedPoolRequestReader, PoolRequestBodyPreparationError> {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(Box::pin(tokio::io::empty())),
        PoolReplayBodySnapshot::Memory(bytes) => {
            let bytes = bytes.clone();
            let stream = stream::once(async move { Ok::<Bytes, io::Error>(bytes) });
            Ok(Box::pin(StreamReader::new(stream)))
        }
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            let file = tokio::fs::File::open(&temp_file.path)
                .await
                .map_err(|err| {
                    PoolRequestBodyPreparationError::bad_gateway(format!(
                        "failed to open request replay body: {err}"
                    ))
                })?;
            Ok(Box::pin(file))
        }
    }
}

async fn decode_pool_request_reader(
    reader: BoxedPoolRequestReader,
    encoding: RequestBodyContentEncoding,
) -> Result<BoxedPoolRequestReader, PoolRequestBodyPreparationError> {
    match encoding {
        RequestBodyContentEncoding::Identity => Ok(reader),
        RequestBodyContentEncoding::Gzip => Ok(Box::pin(AsyncGzipDecoder::new(
            tokio::io::BufReader::new(reader),
        ))),
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            let mut buffered = tokio::io::BufReader::new(reader);
            let _ = buffered.fill_buf().await.map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to read deflate request body header: {err}"
                ))
            })?;
            if zlib_wrapper {
                Ok(Box::pin(AsyncZlibDecoder::new(buffered)))
            } else {
                Ok(Box::pin(AsyncDeflateDecoder::new(buffered)))
            }
        }
        RequestBodyContentEncoding::Zstd => Ok(Box::pin(AsyncZstdDecoder::new(
            tokio::io::BufReader::new(reader),
        ))),
    }
}

fn encode_pool_request_reader(
    reader: BoxedPoolRequestReader,
    encoding: RequestBodyContentEncoding,
    level: AsyncCompressionLevel,
) -> BoxedPoolRequestReader {
    let buffered = tokio::io::BufReader::new(reader);
    match encoding {
        RequestBodyContentEncoding::Identity => Box::pin(buffered),
        RequestBodyContentEncoding::Gzip => {
            Box::pin(AsyncGzipEncoder::with_quality(buffered, level))
        }
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            if zlib_wrapper {
                Box::pin(AsyncZlibEncoder::with_quality(buffered, level))
            } else {
                Box::pin(
                    async_compression::tokio::bufread::DeflateEncoder::with_quality(
                        buffered, level,
                    ),
                )
            }
        }
        RequestBodyContentEncoding::Zstd => {
            Box::pin(AsyncZstdEncoder::with_quality(buffered, level))
        }
    }
}

fn build_snapshot_upstream_request_body(
    snapshot: &PoolReplayBodySnapshot,
    content_encoding: RequestBodyContentEncoding,
    compression_mode: PoolRequestBodyCompressionMode,
    logical_body_bytes: ObservedBodyBytes,
) -> PreparedPoolUpstreamRequestBody {
    let transmitted_body_bytes = ObservedByteCounter::default();
    PreparedPoolUpstreamRequestBody {
        body: counted_http_body_from_snapshot(snapshot, transmitted_body_bytes.clone()),
        content_length: Some(pool_request_snapshot_body_bytes(snapshot)),
        content_encoding,
        compression_mode,
        byte_observation: PreparedPoolUpstreamRequestBodyObservation {
            logical_body_bytes,
            transmitted_body_bytes,
        },
    }
}

fn build_identity_streaming_upstream_request_body(
    decoded_reader: BoxedPoolRequestReader,
    snapshot: &PoolReplayBodySnapshot,
    snapshot_is_decoded: bool,
) -> PreparedPoolUpstreamRequestBody {
    let transmitted_body_bytes = ObservedByteCounter::default();
    let logical_body_bytes = if snapshot_is_decoded {
        ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(snapshot))
    } else {
        ObservedBodyBytes::Counter(transmitted_body_bytes.clone())
    };
    PreparedPoolUpstreamRequestBody {
        body: counted_http_body_from_reader(decoded_reader, transmitted_body_bytes.clone()),
        content_length: None,
        content_encoding: RequestBodyContentEncoding::Identity,
        compression_mode: PoolRequestBodyCompressionMode::Identity,
        byte_observation: PreparedPoolUpstreamRequestBodyObservation {
            logical_body_bytes,
            transmitted_body_bytes,
        },
    }
}

fn build_recompressed_upstream_request_body(
    decoded_reader: BoxedPoolRequestReader,
    target_encoding: RequestBodyContentEncoding,
    level: AsyncCompressionLevel,
    snapshot: &PoolReplayBodySnapshot,
    snapshot_is_decoded: bool,
) -> PreparedPoolUpstreamRequestBody {
    let logical_body_bytes = if snapshot_is_decoded {
        ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(snapshot))
    } else {
        ObservedBodyBytes::Counter(ObservedByteCounter::default())
    };
    let decoded_reader = match &logical_body_bytes {
        ObservedBodyBytes::Fixed(_) => decoded_reader,
        ObservedBodyBytes::Counter(counter) => {
            Box::pin(CountingAsyncRead::new(decoded_reader, counter.clone()))
        }
    };
    let encoded_reader = encode_pool_request_reader(decoded_reader, target_encoding, level);
    let transmitted_body_bytes = ObservedByteCounter::default();
    PreparedPoolUpstreamRequestBody {
        body: counted_http_body_from_reader(encoded_reader, transmitted_body_bytes.clone()),
        content_length: None,
        content_encoding: target_encoding,
        compression_mode: PoolRequestBodyCompressionMode::Recompressed,
        byte_observation: PreparedPoolUpstreamRequestBodyObservation {
            logical_body_bytes,
            transmitted_body_bytes,
        },
    }
}

pub(crate) async fn build_pool_upstream_request_body(
    prepared: &PreparedPoolRequestBody,
    request_compression_algorithm: RequestCompressionAlgorithm,
    request_compression_level_preset: RequestCompressionLevelPreset,
    downstream_content_encoding: Option<&str>,
) -> Result<PreparedPoolUpstreamRequestBody, PoolRequestBodyPreparationError> {
    if matches!(prepared.snapshot, PoolReplayBodySnapshot::Empty) {
        return Ok(build_snapshot_upstream_request_body(
            &prepared.snapshot,
            RequestBodyContentEncoding::Identity,
            PoolRequestBodyCompressionMode::Identity,
            ObservedBodyBytes::Fixed(0),
        ));
    }

    let downstream_encoding =
        resolve_request_body_content_encoding(&prepared.snapshot, downstream_content_encoding)
            .await?;
    let target_encoding = match request_compression_algorithm {
        RequestCompressionAlgorithm::Follow => downstream_encoding,
        RequestCompressionAlgorithm::Identity => RequestBodyContentEncoding::Identity,
        RequestCompressionAlgorithm::Gzip => RequestBodyContentEncoding::Gzip,
        RequestCompressionAlgorithm::Deflate => {
            RequestBodyContentEncoding::Deflate { zlib_wrapper: true }
        }
        RequestCompressionAlgorithm::Zstd => RequestBodyContentEncoding::Zstd,
    };

    if prepared.snapshot_is_decoded
        && matches!(target_encoding, RequestBodyContentEncoding::Identity)
    {
        return Ok(build_snapshot_upstream_request_body(
            &prepared.snapshot,
            RequestBodyContentEncoding::Identity,
            PoolRequestBodyCompressionMode::Identity,
            ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot)),
        ));
    }

    if !prepared.snapshot_is_decoded && target_encoding == downstream_encoding {
        let compression_mode = if matches!(target_encoding, RequestBodyContentEncoding::Identity) {
            PoolRequestBodyCompressionMode::Identity
        } else {
            PoolRequestBodyCompressionMode::Passthrough
        };
        let logical_body_bytes = if matches!(target_encoding, RequestBodyContentEncoding::Identity)
        {
            ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
        } else {
            ObservedBodyBytes::Fixed(
                count_decoded_request_snapshot_bytes(&prepared.snapshot, target_encoding).await?,
            )
        };
        return Ok(build_snapshot_upstream_request_body(
            &prepared.snapshot,
            target_encoding,
            compression_mode,
            logical_body_bytes,
        ));
    }

    let raw_reader = open_pool_request_snapshot_reader(&prepared.snapshot).await?;
    let decoded_reader = if prepared.snapshot_is_decoded {
        raw_reader
    } else {
        decode_pool_request_reader(raw_reader, downstream_encoding).await?
    };

    if matches!(target_encoding, RequestBodyContentEncoding::Identity) {
        return Ok(build_identity_streaming_upstream_request_body(
            decoded_reader,
            &prepared.snapshot,
            prepared.snapshot_is_decoded,
        ));
    }

    let level = request_compression_preset_to_async_level(request_compression_level_preset);
    Ok(build_recompressed_upstream_request_body(
        decoded_reader,
        target_encoding,
        level,
        &prepared.snapshot,
        prepared.snapshot_is_decoded,
    ))
}

pub(crate) fn pool_request_snapshot_preserves_content_length(
    snapshot: &PoolReplayBodySnapshot,
) -> bool {
    matches!(snapshot, PoolReplayBodySnapshot::File { .. })
}

pub(crate) fn pool_request_snapshot_kind(snapshot: &PoolReplayBodySnapshot) -> &'static str {
    match snapshot {
        PoolReplayBodySnapshot::Empty => "empty",
        PoolReplayBodySnapshot::Memory(_) => "memory",
        PoolReplayBodySnapshot::File { .. } => "file",
    }
}

pub(crate) fn pool_request_snapshot_body_bytes(snapshot: &PoolReplayBodySnapshot) -> usize {
    match snapshot {
        PoolReplayBodySnapshot::Empty => 0,
        PoolReplayBodySnapshot::Memory(bytes) => bytes.len(),
        PoolReplayBodySnapshot::File { size, .. } => *size,
    }
}

pub(crate) fn request_entry_openai_json_tools_contain_image_generation(value: &Value) -> bool {
    value
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.trim() == "image_generation")
            })
        })
}

pub(crate) fn is_openai_responses_lite_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-openai-internal-codex-responses-lite")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexImagegenProtocol {
    Full,
    Lite,
}

impl CodexImagegenProtocol {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "responses_full",
            Self::Lite => "responses_lite",
        }
    }
}

pub(crate) fn codex_imagegen_protocol_from_headers(
    headers: &HeaderMap,
) -> Option<CodexImagegenProtocol> {
    if is_openai_responses_lite_request(headers) {
        return Some(CodexImagegenProtocol::Lite);
    }

    let originator_matches = headers
        .get("originator")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("Codex Desktop"));
    let user_agent_matches = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().starts_with("Codex Desktop/"));
    (originator_matches || user_agent_matches).then_some(CodexImagegenProtocol::Full)
}

const CODEX_IMAGEGEN_NAMESPACE: &str = "image_gen";
const CODEX_IMAGEGEN_TOOL: &str = "imagegen";
const CODEX_IMAGEGEN_DESCRIPTION: &str = r#"The `image_gen.imagegen` tool enables image generation from descriptions and editing of existing images based on specific instructions. Use it when:

- The user requests an image based on a scene description, such as a diagram, portrait, comic, meme, or any other visual.
- The user wants to modify an attached or previously generated image with specific changes, including adding or removing elements, altering colors, improving quality/resolution, or transforming the style (e.g., cartoon, oil painting).

Guidelines:
- imagegen needs a few minutes to finish. In code-mode, use the first-line @exec directive to give the initial call 120 seconds and the same yield for any waits that follow. Once it finishes, return the image with generatedImage(result).
- Omit both `referenced_image_paths` and `num_last_images_to_include` when generating a brand new image.
- For edits, use `referenced_image_paths` when every target image has a local file path.
- If you have not seen a local image yet, use `view_image` to inspect it before editing.
- Use `num_last_images_to_include` only when at least one target image has no local file path.
- Set `num_last_images_to_include` to the smallest number of recent conversation images that includes every target image, up to 5.
- Never provide both `referenced_image_paths` and `num_last_images_to_include`.
- If neither mechanism can include every target image, ask the user to attach the missing images again.
- Directly generate the image without reconfirmation or clarification unless required images must be attached again.
- Always use this tool for image editing unless the user explicitly requests otherwise. Do not use the `python` tool for image editing unless specifically instructed."#;

fn codex_imagegen_function() -> Value {
    serde_json::json!({
        "type": "function",
        "name": CODEX_IMAGEGEN_TOOL,
        "description": CODEX_IMAGEGEN_DESCRIPTION,
        "strict": false,
        "parameters": {
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "referenced_image_paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 5
                },
                "num_last_images_to_include": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        }
    })
}

fn codex_imagegen_namespace() -> Value {
    serde_json::json!({
        "type": "namespace",
        "name": CODEX_IMAGEGEN_NAMESPACE,
        "description": "Tools in the image_gen namespace.",
        "tools": [codex_imagegen_function()]
    })
}

fn is_codex_imagegen_namespace(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("namespace")
        && tool.get("name").and_then(Value::as_str) == Some(CODEX_IMAGEGEN_NAMESPACE)
}

fn is_codex_imagegen_function(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("function")
        && tool.get("name").and_then(Value::as_str) == Some(CODEX_IMAGEGEN_TOOL)
}

fn is_legacy_codex_imagegen_tool(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("image_gen.imagegen")
}

fn find_codex_imagegen_function(tools: &[Value]) -> Option<Value> {
    tools.iter().find_map(|tool| {
        if is_legacy_codex_imagegen_tool(tool) {
            return Some(tool.clone());
        }
        if is_codex_imagegen_namespace(tool) {
            return tool
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|namespace_tools| {
                    namespace_tools
                        .iter()
                        .find(|tool| is_codex_imagegen_function(tool))
                        .cloned()
                });
        }
        None
    })
}

fn codex_imagegen_schema_fingerprint(tool: &Value) -> String {
    serde_json::to_string(tool)
        .map(|value| short_sha256_fingerprint(&value))
        .unwrap_or_else(|_| "unavailable".to_string())
}

fn codex_imagegen_schema_diff_paths(
    before: &Value,
    after: &Value,
    path: &str,
    paths: &mut Vec<String>,
) {
    if paths.len() >= 16 {
        return;
    }
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            for key in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
                let next = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                match (before.get(key), after.get(key)) {
                    (Some(before), Some(after)) => {
                        codex_imagegen_schema_diff_paths(before, after, &next, paths)
                    }
                    _ => paths.push(next),
                }
                if paths.len() >= 16 {
                    break;
                }
            }
        }
        _ if before != after => paths.push(path.to_string()),
        _ => {}
    }
}

fn remove_codex_imagegen_from_tool_list(tools: &mut Vec<Value>) -> bool {
    let mut removed = false;
    for namespace in tools
        .iter_mut()
        .filter(|tool| is_codex_imagegen_namespace(tool))
    {
        if let Some(namespace_tools) = namespace.get_mut("tools").and_then(Value::as_array_mut) {
            let original_len = namespace_tools.len();
            namespace_tools.retain(|tool| !is_codex_imagegen_function(tool));
            removed |= namespace_tools.len() != original_len;
        }
    }
    let original_len = tools.len();
    tools.retain(|tool| {
        if is_legacy_codex_imagegen_tool(tool) {
            return false;
        }
        !is_codex_imagegen_namespace(tool)
            || tool
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
    });
    removed || tools.len() != original_len
}

fn inject_missing_codex_imagegen_tool(tools: &mut Vec<Value>) {
    let replacement = codex_imagegen_function();
    if let Some(namespace) = tools
        .iter_mut()
        .find(|tool| is_codex_imagegen_namespace(tool))
    {
        let Some(namespace_tools) = namespace.get_mut("tools").and_then(Value::as_array_mut) else {
            *namespace = codex_imagegen_namespace();
            return;
        };
        namespace_tools.push(replacement);
    } else {
        tools.push(codex_imagegen_namespace());
    }
}

fn find_codex_imagegen_target_namespace(
    tools: &[Value],
    replacement: &Value,
) -> (Option<usize>, bool) {
    let mut first_function_namespace = None;
    let mut first_function = None;
    let mut function_count = 0;
    for (namespace_index, namespace) in tools.iter().enumerate() {
        if !is_codex_imagegen_namespace(namespace) {
            continue;
        }
        for function in namespace
            .get("tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|tool| is_codex_imagegen_function(tool))
        {
            if first_function.is_none() {
                first_function_namespace = Some(namespace_index);
                first_function = Some(function);
            }
            function_count += 1;
        }
    }
    let target_namespace_index =
        first_function_namespace.or_else(|| tools.iter().position(is_codex_imagegen_namespace));
    let already_canonical =
        function_count == 1 && first_function.is_some_and(|function| function == replacement);
    (target_namespace_index, already_canonical)
}

fn normalize_codex_imagegen_namespace(namespace: &mut Value, replacement: &Value) {
    let Some(namespace_tools) = namespace.get_mut("tools").and_then(Value::as_array_mut) else {
        *namespace = codex_imagegen_namespace();
        return;
    };
    let mut canonical_inserted = false;
    let mut normalized_tools = Vec::with_capacity(namespace_tools.len() + 1);
    for tool in namespace_tools.drain(..) {
        if is_codex_imagegen_function(&tool) {
            if !canonical_inserted {
                normalized_tools.push(replacement.clone());
                canonical_inserted = true;
            }
        } else {
            normalized_tools.push(tool);
        }
    }
    if !canonical_inserted {
        normalized_tools.push(replacement.clone());
    }
    *namespace_tools = normalized_tools;
}

fn normalize_codex_imagegen_tool_list(
    tools: &mut Vec<Value>,
    replacement: &Value,
    target_namespace_index: usize,
) {
    for (namespace_index, namespace) in tools.iter_mut().enumerate() {
        if !is_codex_imagegen_namespace(namespace) {
            continue;
        }
        if namespace_index == target_namespace_index {
            normalize_codex_imagegen_namespace(namespace, replacement);
        } else if let Some(namespace_tools) =
            namespace.get_mut("tools").and_then(Value::as_array_mut)
        {
            namespace_tools.retain(|tool| !is_codex_imagegen_function(tool));
        }
    }
}

fn replace_codex_imagegen_in_tool_list(
    tools: &mut Vec<Value>,
    mode: crate::CodexImagegenRewriteMode,
) -> (bool, Option<Value>, &'static str) {
    use crate::CodexImagegenRewriteMode::*;

    let existing = find_codex_imagegen_function(tools);
    match mode {
        KeepOriginal => (false, existing, "no_change"),
        ForceRemove => {
            let removed = remove_codex_imagegen_from_tool_list(tools);
            let outcome = if removed { "removed" } else { "no_change" };
            (removed, existing, outcome)
        }
        FillMissing if existing.is_some() => (false, existing, "no_change"),
        FillMissing => {
            inject_missing_codex_imagegen_tool(tools);
            (true, existing, "injected")
        }
        ForceAdd => {
            let replacement = codex_imagegen_function();
            let original_len = tools.len();
            tools.retain(|tool| !is_legacy_codex_imagegen_tool(tool));
            let mut updated = tools.len() != original_len;
            let (target_namespace_index, already_canonical) =
                find_codex_imagegen_target_namespace(tools, &replacement);
            if let Some(target_namespace_index) = target_namespace_index {
                if !already_canonical {
                    normalize_codex_imagegen_tool_list(tools, &replacement, target_namespace_index);
                    updated = true;
                }
            } else {
                tools.push(codex_imagegen_namespace());
                updated = true;
            }

            let outcome = if existing.is_some() && updated {
                "replaced"
            } else if updated {
                "injected"
            } else {
                "no_change"
            };
            (updated, existing, outcome)
        }
    }
}
fn remove_hosted_image_generation(value: &mut Value) -> bool {
    fn remove_from_tools(tools: &mut Vec<Value>) -> bool {
        let original_len = tools.len();
        tools.retain(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .is_none_or(|tool_type| tool_type.trim() != "image_generation")
        });
        tools.len() != original_len
    }

    let tool_choice_selects_image_generation =
        request_entry_openai_json_tool_choice_selects_image_generation(value);
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let mut modified = false;
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        modified |= remove_from_tools(tools);
    }
    match object.get_mut("input") {
        Some(Value::Object(input)) => {
            if let Some(tools) = input
                .get_mut("additional_tools")
                .and_then(Value::as_array_mut)
            {
                modified |= remove_from_tools(tools);
            }
        }
        Some(Value::Array(input)) => {
            for item in input {
                if is_lite_developer_additional_tools(item)
                    && let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut)
                {
                    modified |= remove_from_tools(tools);
                }
            }
        }
        _ => {}
    }
    if tool_choice_selects_image_generation {
        object.remove("tool_choice");
        modified = true;
    }
    modified
}

fn normalise_lite_input_tools(value: &mut Value) -> Option<(&mut Vec<Value>, bool)> {
    let object = value.as_object_mut()?;
    let mut normalized = false;
    if !object.get("input").is_some_and(Value::is_array) {
        let original = object.remove("input").unwrap_or(Value::Null);
        let mut input = Vec::new();
        if let Value::Object(mut original) = original {
            if let Some(tools) = original.remove("additional_tools")
                && let Some(tools) = tools.as_array()
                && !tools.is_empty()
            {
                input.push(serde_json::json!({
                    "type": "additional_tools",
                    "role": "developer",
                    "tools": tools,
                }));
            }
            if !original.is_empty() {
                input.push(serde_json::json!({
                    "type": "message",
                    "role": "user",
                    "content": original,
                }));
            }
        } else if !original.is_null() {
            input.push(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": original,
            }));
        }
        object.insert("input".to_string(), Value::Array(input));
        normalized = true;
    }
    object
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .map(|input| (input, normalized))
}

fn take_lite_top_level_developer_tools(value: &mut Value) -> (Vec<Value>, bool) {
    let Some(tools) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("tools"))
        .and_then(Value::as_array_mut)
    else {
        return (Vec::new(), false);
    };

    let mut developer_tools = Vec::new();
    let mut retained = Vec::with_capacity(tools.len());
    for tool in std::mem::take(tools) {
        if tool.get("type").and_then(Value::as_str) == Some("namespace")
            || is_legacy_codex_imagegen_tool(&tool)
        {
            developer_tools.push(tool);
        } else {
            retained.push(tool);
        }
    }
    let moved = !developer_tools.is_empty();
    *tools = retained;
    (developer_tools, moved)
}

fn is_lite_developer_additional_tools(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("additional_tools")
        && item
            .get("role")
            .and_then(Value::as_str)
            .is_none_or(|role| role.eq_ignore_ascii_case("developer"))
}

fn ensure_lite_codex_execution_contract(value: &mut Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let reasoning = object
        .entry("reasoning".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    if !reasoning.is_object() {
        *reasoning = Value::Object(Default::default());
        changed = true;
    }
    let reasoning = reasoning
        .as_object_mut()
        .expect("reasoning was normalized to an object");
    if reasoning.get("context").and_then(Value::as_str) != Some("all_turns") {
        reasoning.insert(
            "context".to_string(),
            Value::String("all_turns".to_string()),
        );
        changed = true;
    }
    if object.get("parallel_tool_calls").and_then(Value::as_bool) != Some(false) {
        object.insert("parallel_tool_calls".to_string(), Value::Bool(false));
        changed = true;
    }
    changed
}

fn find_lite_codex_imagegen_function(
    input: &[Value],
    top_level_developer_tools: &[Value],
) -> Option<Value> {
    input
        .iter()
        .find_map(|item| {
            is_lite_developer_additional_tools(item)
                .then(|| item.get("tools").and_then(Value::as_array))
                .flatten()
                .and_then(|tools| find_codex_imagegen_function(tools.as_slice()))
        })
        .or_else(|| find_codex_imagegen_function(top_level_developer_tools))
}

fn ensure_lite_developer_tools_position(
    input: &mut Vec<Value>,
    top_level_developer_tools: &[Value],
    mode: crate::CodexImagegenRewriteMode,
    existing: Option<&Value>,
) -> Option<usize> {
    input
        .iter()
        .position(is_lite_developer_additional_tools)
        .or_else(|| {
            let needs_developer_tools = !top_level_developer_tools.is_empty()
                || matches!(mode, crate::CodexImagegenRewriteMode::ForceAdd)
                || (matches!(mode, crate::CodexImagegenRewriteMode::FillMissing)
                    && existing.is_none());
            needs_developer_tools.then(|| {
                input.insert(
                    0,
                    serde_json::json!({
                        "type": "additional_tools",
                        "role": "developer",
                        "tools": [],
                    }),
                );
                0
            })
        })
}

fn merge_lite_top_level_developer_tools(
    input: &mut [Value],
    position: usize,
    top_level_developer_tools: Vec<Value>,
) -> bool {
    let tools = input[position]
        .as_object_mut()
        .expect("additional tools item is an object")
        .entry("tools".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let mut changed = false;
    if !tools.is_array() {
        *tools = Value::Array(Vec::new());
        changed = true;
    }
    if let Some(tools) = tools.as_array_mut() {
        tools.extend(top_level_developer_tools);
    }
    changed
}

fn remove_lite_codex_imagegen_tools(input: &mut [Value]) -> bool {
    let mut removed = false;
    for item in input
        .iter_mut()
        .filter(|item| is_lite_developer_additional_tools(item))
    {
        if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
            let (tool_changed, _, _) = replace_codex_imagegen_in_tool_list(
                tools,
                crate::CodexImagegenRewriteMode::ForceRemove,
            );
            removed |= tool_changed;
        }
    }
    removed
}

fn force_add_lite_codex_imagegen_tool(
    input: &mut [Value],
    target_position: usize,
    existing: Option<&Value>,
    changed: &mut bool,
) -> &'static str {
    let target_already_canonical = input[target_position]
        .get("tools")
        .and_then(Value::as_array)
        .and_then(|tools| find_codex_imagegen_function(tools))
        .is_some_and(|tool| tool == codex_imagegen_function());
    let multiple_definitions = input
        .iter()
        .filter(|item| is_lite_developer_additional_tools(item))
        .filter_map(|item| item.get("tools").and_then(Value::as_array))
        .filter(|tools| find_codex_imagegen_function(tools).is_some())
        .count()
        > 1;
    if !target_already_canonical || multiple_definitions {
        *changed |= remove_lite_codex_imagegen_tools(input);
        let tools = input[target_position]
            .get_mut("tools")
            .and_then(Value::as_array_mut)
            .expect("developer tools are normalized");
        let (tool_changed, _, _) =
            replace_codex_imagegen_in_tool_list(tools, crate::CodexImagegenRewriteMode::ForceAdd);
        *changed |= tool_changed;
    }
    if existing.is_some() && !target_already_canonical {
        "replaced"
    } else if *changed {
        "injected"
    } else {
        "no_change"
    }
}

fn rewrite_lite_codex_imagegen_mode(
    input: &mut [Value],
    first_developer_tools_position: Option<usize>,
    mode: crate::CodexImagegenRewriteMode,
    existing: Option<&Value>,
    changed: &mut bool,
) -> &'static str {
    match mode {
        crate::CodexImagegenRewriteMode::KeepOriginal => "no_change",
        crate::CodexImagegenRewriteMode::FillMissing if existing.is_some() => "no_change",
        crate::CodexImagegenRewriteMode::FillMissing => {
            let position =
                first_developer_tools_position.expect("fill missing creates developer tools");
            let tools = input[position]
                .get_mut("tools")
                .and_then(Value::as_array_mut)
                .expect("developer tools are normalized");
            let (tool_changed, _, outcome) = replace_codex_imagegen_in_tool_list(tools, mode);
            *changed |= tool_changed;
            outcome
        }
        crate::CodexImagegenRewriteMode::ForceRemove => {
            let removed = remove_lite_codex_imagegen_tools(input);
            *changed |= removed;
            if removed { "removed" } else { "no_change" }
        }
        crate::CodexImagegenRewriteMode::ForceAdd => {
            let position =
                first_developer_tools_position.expect("force add creates developer tools");
            force_add_lite_codex_imagegen_tool(input, position, existing, changed)
        }
    }
}

fn rewrite_lite_codex_imagegen_tools(
    value: &mut Value,
    mode: crate::CodexImagegenRewriteMode,
) -> (bool, Option<Value>, &'static str) {
    let (top_level_developer_tools, migrated_top_level_developer_tools) =
        take_lite_top_level_developer_tools(value);
    let Some((mut input, input_normalized)) = normalise_lite_input_tools(value) else {
        return (false, None, "invalid_input");
    };
    let existing = find_lite_codex_imagegen_function(&input, &top_level_developer_tools);
    let first_developer_tools_position = ensure_lite_developer_tools_position(
        input,
        &top_level_developer_tools,
        mode,
        existing.as_ref(),
    );
    let mut changed = input_normalized || migrated_top_level_developer_tools;
    if let Some(position) = first_developer_tools_position {
        changed |= merge_lite_top_level_developer_tools(input, position, top_level_developer_tools);
    }
    let outcome = rewrite_lite_codex_imagegen_mode(
        input,
        first_developer_tools_position,
        mode,
        existing.as_ref(),
        &mut changed,
    );

    (changed, existing, outcome)
}

fn codex_imagegen_audit(
    protocol: CodexImagegenProtocol,
    mode: crate::CodexImagegenRewriteMode,
    outcome: &str,
    existing: Option<&Value>,
    hosted_removed: bool,
    reason: Option<&str>,
) -> Value {
    let canonical = codex_imagegen_function();
    let mut audit = serde_json::json!({
        "protocol": protocol.as_str(),
        "clientMatch": "codex_desktop",
        "mode": mode.as_str(),
        "outcome": outcome,
        "hostedRemoved": hosted_removed,
        "snapshotCommit": "61a44880a85d2fd0d8770908dea5733495e571c8",
        "injectedSchemaFingerprint": codex_imagegen_schema_fingerprint(&canonical),
    });
    if let Some(existing) = existing {
        audit["existingSchemaFingerprint"] =
            Value::String(codex_imagegen_schema_fingerprint(existing));
        let mut diff_paths = Vec::new();
        codex_imagegen_schema_diff_paths(existing, &canonical, "", &mut diff_paths);
        audit["schemaDiffPaths"] = serde_json::json!(diff_paths);
    }
    if let Some(reason) = reason {
        audit["reason"] = Value::String(reason.to_string());
    }
    audit
}

pub(crate) fn codex_imagegen_keep_original_audit(protocol: CodexImagegenProtocol) -> Value {
    codex_imagegen_audit(
        protocol,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        "no_change",
        None,
        false,
        Some("policy_keep_original"),
    )
}

pub(crate) fn codex_imagegen_upstream_incompatibility(
    status: StatusCode,
    message: &str,
    audit: Option<&Value>,
) -> bool {
    if status != StatusCode::BAD_GATEWAY {
        return false;
    }
    codex_imagegen_audit_has_canonical_namespace(audit)
        && message
            .to_ascii_lowercase()
            .contains("upstream request failed")
}

pub(crate) fn codex_imagegen_audit_was_injected(audit: Option<&Value>) -> bool {
    audit
        .and_then(|value| value.get("outcome"))
        .and_then(Value::as_str)
        .is_some_and(|outcome| matches!(outcome, "injected" | "replaced"))
}

pub(crate) fn codex_imagegen_audit_has_canonical_namespace(audit: Option<&Value>) -> bool {
    if codex_imagegen_audit_was_injected(audit) {
        return true;
    }
    let Some(audit) = audit else {
        return false;
    };
    audit.get("outcome").and_then(Value::as_str) == Some("no_change")
        && matches!(
            audit.get("mode").and_then(Value::as_str),
            Some("force_add" | "fill_missing")
        )
        && audit.get("reason").and_then(Value::as_str) == Some("already_current")
        && audit
            .get("schemaDiffPaths")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
}

fn rewrite_codex_imagegen_tools(
    value: &mut Value,
    protocol: CodexImagegenProtocol,
    mode: crate::CodexImagegenRewriteMode,
    image_intent: crate::ImageIntent,
) -> (bool, Value) {
    use crate::CodexImagegenRewriteMode::*;

    if matches!(mode, KeepOriginal) {
        return (false, codex_imagegen_keep_original_audit(protocol));
    }
    // A non-default Codex policy always supersedes the hosted image tool path.
    let hosted_removed = remove_hosted_image_generation(value);
    if matches!(mode, FillMissing) && image_intent != crate::ImageIntent::Yes {
        return (
            hosted_removed,
            codex_imagegen_audit(
                protocol,
                mode,
                "skipped",
                None,
                hosted_removed,
                Some("image_intent_not_confirmed"),
            ),
        );
    }

    let (changed, existing, outcome) = match protocol {
        CodexImagegenProtocol::Full => {
            let Some(object) = value.as_object_mut() else {
                return (
                    hosted_removed,
                    codex_imagegen_audit(
                        protocol,
                        mode,
                        "invalid_request",
                        None,
                        hosted_removed,
                        None,
                    ),
                );
            };
            if matches!(mode, ForceRemove) {
                match object.get_mut("tools").and_then(Value::as_array_mut) {
                    Some(tools) => replace_codex_imagegen_in_tool_list(tools, mode),
                    None => (false, None, "no_change"),
                }
            } else {
                let tools_value = object
                    .entry("tools".to_string())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if !tools_value.is_array() {
                    *tools_value = Value::Array(Vec::new());
                }
                let tools = tools_value
                    .as_array_mut()
                    .expect("tools was normalized to an array");
                replace_codex_imagegen_in_tool_list(tools, mode)
            }
        }
        CodexImagegenProtocol::Lite => {
            let (changed, existing, outcome) = rewrite_lite_codex_imagegen_tools(value, mode);
            let execution_contract_changed = ensure_lite_codex_execution_contract(value);
            (changed || execution_contract_changed, existing, outcome)
        }
    };
    let reason = (outcome == "no_change").then_some("already_current");
    let audit = codex_imagegen_audit(
        protocol,
        mode,
        outcome,
        existing.as_ref(),
        hosted_removed,
        reason,
    );
    if outcome == "replaced" {
        warn!(
            protocol = protocol.as_str(),
            mode = mode.as_str(),
            outcome,
            hosted_removed,
            existing_schema_fingerprint = ?audit.get("existingSchemaFingerprint"),
            injected_schema_fingerprint = ?audit.get("injectedSchemaFingerprint"),
            schema_diff_paths = ?audit.get("schemaDiffPaths"),
            "replaced conflicting Codex imagegen tool schema"
        );
    } else if changed || hosted_removed {
        info!(
            protocol = protocol.as_str(),
            mode = mode.as_str(),
            outcome,
            hosted_removed,
            existing_schema = existing.is_some(),
            injected_schema_fingerprint = ?audit.get("injectedSchemaFingerprint"),
            reason = ?audit.get("reason"),
            "rewrote Codex imagegen tool contract"
        );
    }
    (changed || hosted_removed, audit)
}

pub(crate) fn image_tool_rewrite_audit(
    target: ProxyCaptureTarget,
    responses_lite: bool,
    mode: crate::ImageToolRewriteMode,
) -> Option<Value> {
    if !matches!(
        target,
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
    ) {
        return None;
    }

    let (protocol, outcome, reason) = if responses_lite {
        (
            "responses_lite",
            "skipped",
            Some("responses_lite_client_owned_tools"),
        )
    } else if mode == crate::ImageToolRewriteMode::KeepOriginal {
        ("responses_full", "no_change", None)
    } else {
        ("responses_full", "applied", None)
    };
    let mut audit = serde_json::json!({
        "protocol": protocol,
        "mode": mode.as_str(),
        "outcome": outcome,
    });
    if let Some(reason) = reason {
        audit["reason"] = Value::String(reason.to_string());
    }
    Some(audit)
}

pub(crate) fn request_entry_openai_json_tool_choice_selects_image_generation(
    value: &Value,
) -> bool {
    let Some(tool_choice) = value.get("tool_choice") else {
        return false;
    };
    match tool_choice {
        Value::String(choice) => choice.trim() == "image_generation",
        Value::Object(choice) => {
            choice
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|tool_type| tool_type.trim() == "image_generation")
                || choice
                    .get("tool")
                    .and_then(Value::as_object)
                    .and_then(|tool| tool.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.trim() == "image_generation")
                || choice
                    .get("function")
                    .and_then(Value::as_object)
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.trim() == "image_generation")
        }
        _ => false,
    }
}

pub(crate) fn rewrite_openai_responses_image_tools(
    value: &mut Value,
    rewrite_mode: crate::ImageToolRewriteMode,
    image_intent: crate::ImageIntent,
) -> bool {
    use crate::ImageToolRewriteMode::*;

    let has_image_tool = request_entry_openai_json_tools_contain_image_generation(value);
    let tool_choice_selects_image_generation =
        request_entry_openai_json_tool_choice_selects_image_generation(value);
    let Some(obj) = value.as_object_mut() else {
        return false;
    };

    match rewrite_mode {
        KeepOriginal => false,
        ForceRemove => {
            let mut modified = false;
            if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
                let original_len = tools.len();
                tools.retain(|tool| {
                    tool.get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|tool_type| tool_type.trim() != "image_generation")
                });
                modified |= tools.len() != original_len;
            }
            if tool_choice_selects_image_generation {
                obj.remove("tool_choice");
                modified = true;
            }
            modified
        }
        FillMissing | ForceAdd => {
            if matches!(rewrite_mode, FillMissing) && image_intent != crate::ImageIntent::Yes {
                return false;
            }

            let mut modified = false;
            if !has_image_tool {
                let tool = serde_json::json!({
                    "type": "image_generation",
                    "output_format": "png",
                });
                match obj.get_mut("tools") {
                    Some(Value::Array(tools)) => {
                        tools.push(tool);
                    }
                    Some(_) => {
                        obj.insert("tools".to_string(), Value::Array(vec![tool]));
                    }
                    None => {
                        obj.insert("tools".to_string(), Value::Array(vec![tool]));
                    }
                }
                modified = true;
            }
            if !obj.contains_key("tool_choice") {
                obj.insert(
                    "tool_choice".to_string(),
                    serde_json::json!({"type": "image_generation"}),
                );
                modified = true;
            }
            modified
        }
    }
}

pub(crate) async fn prepare_pool_request_body_for_account(
    proxy_request_id: u64,
    body: Option<&PoolReplayBodySnapshot>,
    original_uri: &Uri,
    method: &Method,
    content_encoding: Option<&str>,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
    image_tool_rewrite_mode: crate::ImageToolRewriteMode,
    codex_imagegen_rewrite_mode: crate::CodexImagegenRewriteMode,
    codex_imagegen_protocol: Option<CodexImagegenProtocol>,
    projected_request_info: Option<&RequestCaptureInfo>,
    projected_hosted_image_intent: Option<ImageIntent>,
    model_mapping: Option<&ResolvedModelMapping>,
) -> Result<PreparedPoolRequestBody, PoolRequestBodyPreparationError> {
    let capture_target = capture_target_for_request(original_uri.path(), method);
    let default_image_intent = match capture_target {
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            ImageIntent::DirectImage
        }
        _ => ImageIntent::Unknown,
    };
    let fast_mode_rewrite_required = capture_target
        .is_some_and(|target| target.allows_fast_mode_rewrite())
        && fast_mode_rewrite_mode != TagFastModeRewriteMode::KeepOriginal;
    let codex_imagegen_rewrite_required = capture_target.is_some_and(|target| {
        matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        )
    }) && codex_imagegen_protocol.is_some()
        && codex_imagegen_rewrite_mode != crate::CodexImagegenRewriteMode::KeepOriginal;
    let image_tool_rewrite_required = capture_target.is_some_and(|target| {
        matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        )
    }) && codex_imagegen_protocol.is_none()
        && image_tool_rewrite_mode != crate::ImageToolRewriteMode::KeepOriginal;
    let model_mapping_required = model_mapping.is_some();
    let rewrite_required = model_mapping_required
        || fast_mode_rewrite_required
        || image_tool_rewrite_required
        || codex_imagegen_rewrite_required;

    let Some(snapshot) = body.cloned() else {
        if model_mapping_required {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a JSON request body with a top-level model field",
            ));
        }
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Empty,
            request_body_for_capture: Some(Bytes::new()),
            requested_service_tier: None,
            requested_image_intent: default_image_intent,
            requested_hosted_image_intent: default_image_intent,
            codex_imagegen_rewrite: codex_imagegen_protocol
                .filter(|_| {
                    codex_imagegen_rewrite_mode == crate::CodexImagegenRewriteMode::KeepOriginal
                })
                .map(codex_imagegen_keep_original_audit),
            snapshot_is_decoded: false,
        });
    };

    if !rewrite_required {
        let (
            request_body_for_capture,
            requested_service_tier,
            requested_image_intent,
            requested_hosted_image_intent,
        ) = match &snapshot {
            PoolReplayBodySnapshot::Empty => (
                Some(Bytes::new()),
                None,
                default_image_intent,
                default_image_intent,
            ),
            PoolReplayBodySnapshot::Memory(bytes) => {
                let (requested_service_tier, requested_image_intent, requested_hosted_image_intent) =
                    projected_request_info
                        .zip(projected_hosted_image_intent)
                        .map(|(info, hosted_image_intent)| {
                            let image_intent = info
                                .image_intent
                                .as_deref()
                                .map(ImageIntent::from_str)
                                .unwrap_or(default_image_intent);
                            (
                                info.requested_service_tier.clone(),
                                image_intent,
                                hosted_image_intent,
                            )
                        })
                        .or_else(|| {
                            serde_json::from_slice::<Value>(bytes).ok().map(|value| {
                                (
                                    extract_requested_service_tier_from_request_body(&value),
                                    capture_target
                                        .map(|target| {
                                            infer_image_intent_from_request_body(target, &value)
                                        })
                                        .unwrap_or(ImageIntent::Unknown),
                                    capture_target
                                        .map(|target| {
                                            infer_hosted_image_intent_from_request_body(
                                                target, &value,
                                            )
                                        })
                                        .unwrap_or(ImageIntent::Unknown),
                                )
                            })
                        })
                        .unwrap_or((None, default_image_intent, default_image_intent));
                (
                    Some(bytes.clone()),
                    requested_service_tier,
                    requested_image_intent,
                    requested_hosted_image_intent,
                )
            }
            PoolReplayBodySnapshot::File { .. } => {
                let requested_service_tier =
                    projected_request_info.and_then(|info| info.requested_service_tier.clone());
                let image_intent = projected_request_info
                    .and_then(|info| info.image_intent.as_deref())
                    .map(ImageIntent::from_str)
                    .unwrap_or(default_image_intent);
                (
                    None,
                    requested_service_tier,
                    image_intent,
                    projected_hosted_image_intent.unwrap_or(default_image_intent),
                )
            }
        };
        let codex_imagegen_rewrite = codex_imagegen_protocol
            .filter(|_| {
                codex_imagegen_rewrite_mode == crate::CodexImagegenRewriteMode::KeepOriginal
            })
            .map(codex_imagegen_keep_original_audit);
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture,
            requested_service_tier,
            requested_image_intent,
            requested_hosted_image_intent,
            codex_imagegen_rewrite,
            snapshot_is_decoded: false,
        });
    }

    let original_bytes = snapshot.to_bytes().await.map_err(|err| {
        PoolRequestBodyPreparationError::bad_gateway(format!(
            "failed to materialize pool request body for rewrite: {err}"
        ))
    })?;
    info!(
        proxy_request_id,
        json_parse_count = 1_u8,
        whole_body_materialization_count = 1_u8,
        materialization_bytes = original_bytes.len(),
        purpose = "account_specific_request_rewrite",
        "pool request preparation materialized account-specific rewrite body"
    );
    let downstream_encoding =
        resolve_request_body_content_encoding(&snapshot, content_encoding).await?;
    let decoded_original_bytes =
        decode_request_payload_bytes(&original_bytes, downstream_encoding)?;
    let Some(target) = capture_target else {
        if model_mapping_required {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping is not supported for this request endpoint",
            ));
        }
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture: Some(original_bytes),
            requested_service_tier: None,
            requested_image_intent: default_image_intent,
            requested_hosted_image_intent: default_image_intent,
            codex_imagegen_rewrite: None,
            snapshot_is_decoded: false,
        });
    };
    let mut value = match serde_json::from_slice::<Value>(&decoded_original_bytes) {
        Ok(value) => value,
        Err(_) if model_mapping_required => {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a valid JSON request body",
            ));
        }
        Err(_) => {
            return Ok(PreparedPoolRequestBody {
                snapshot,
                request_body_for_capture: Some(original_bytes),
                requested_service_tier: None,
                requested_image_intent: default_image_intent,
                requested_hosted_image_intent: default_image_intent,
                codex_imagegen_rewrite: None,
                snapshot_is_decoded: false,
            });
        }
    };

    let model_mapping_rewritten = if let Some(mapping) = model_mapping {
        let Some(object) = value.as_object_mut() else {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a JSON object with a top-level model field",
            ));
        };
        if object.get("model").and_then(Value::as_str).is_none() {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a top-level string model field",
            ));
        }
        object.insert(
            "model".to_string(),
            Value::String(mapping.target_model.clone()),
        );
        true
    } else {
        false
    };
    let rewritten = model_mapping_rewritten
        || (if target.allows_fast_mode_rewrite() {
            rewrite_request_service_tier_for_fast_mode(&mut value, fast_mode_rewrite_mode)
        } else {
            false
        });
    let original_image_intent = infer_image_intent_from_request_body(target, &value);
    let (codex_image_rewritten, codex_imagegen_rewrite) = if let Some(protocol) =
        codex_imagegen_protocol
        && matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        ) {
        let (rewritten, audit) = rewrite_codex_imagegen_tools(
            &mut value,
            protocol,
            codex_imagegen_rewrite_mode,
            original_image_intent,
        );
        (rewritten, Some(audit))
    } else {
        (false, None)
    };
    let image_rewritten = if codex_imagegen_protocol.is_none()
        && matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        ) {
        rewrite_openai_responses_image_tools(
            &mut value,
            image_tool_rewrite_mode,
            original_image_intent,
        )
    } else {
        false
    };
    let requested_service_tier = extract_requested_service_tier_from_request_body(&value);
    let upstream_image_intent = infer_image_intent_from_request_body(target, &value);
    let upstream_hosted_image_intent = infer_hosted_image_intent_from_request_body(target, &value);
    if !rewritten && !image_rewritten && !codex_image_rewritten {
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture: Some(original_bytes),
            requested_service_tier,
            requested_image_intent: upstream_image_intent,
            requested_hosted_image_intent: upstream_hosted_image_intent,
            codex_imagegen_rewrite,
            snapshot_is_decoded: false,
        });
    }

    let rewritten_bytes = serde_json::to_vec(&value).map(Bytes::from).map_err(|err| {
        PoolRequestBodyPreparationError::bad_gateway(format!(
            "failed to serialize rewritten pool request body: {err}"
        ))
    })?;
    let rewritten_snapshot =
        pool_replay_snapshot_from_bytes(proxy_request_id, rewritten_bytes.clone())
            .await
            .map_err(|err| {
                PoolRequestBodyPreparationError::bad_gateway(format!(
                    "failed to persist rewritten pool request body: {err}"
                ))
            })?;
    Ok(PreparedPoolRequestBody {
        snapshot: rewritten_snapshot,
        request_body_for_capture: Some(rewritten_bytes.clone()),
        requested_service_tier,
        requested_image_intent: upstream_image_intent,
        requested_hosted_image_intent: upstream_hosted_image_intent,
        codex_imagegen_rewrite,
        snapshot_is_decoded: true,
    })
}
const INCLUDE_USAGE_ROOT_INSERTION: &[u8] = br#","stream_options":{"include_usage":true}"#;
const INCLUDE_USAGE_OBJECT_INSERTION: &[u8] = br#","include_usage":true"#;
const INCLUDE_USAGE_EMPTY_OBJECT_INSERTION: &[u8] = br#""include_usage":true"#;
const INCLUDE_USAGE_REPLACEMENT: &[u8] = br#"{"include_usage":true}"#;
const INCLUDE_USAGE_COPY_BUFFER_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy)]
enum IncludeUsageRewritePlan {
    InsertRoot {
        offset: usize,
    },
    InsertObject {
        start: usize,
        offset: usize,
        empty: bool,
    },
    ReplaceValue {
        start: usize,
        end: usize,
    },
    ReplaceIncludeUsageValue {
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, Copy)]
enum ActiveStreamOptionsValue {
    Object {
        start: usize,
        depth: usize,
        has_content: bool,
    },
    Composite {
        start: usize,
        depth: usize,
    },
    Primitive {
        start: usize,
        last_non_whitespace: usize,
    },
    String {
        start: usize,
    },
}

fn locate_root_field_rewrite(
    reader: impl Read,
    target_key: &[u8],
) -> io::Result<Option<IncludeUsageRewritePlan>> {
    let mut reader = std::io::BufReader::with_capacity(INCLUDE_USAGE_COPY_BUFFER_BYTES, reader);
    let mut depth = 0_usize;
    let mut position = 0_usize;
    let mut root_close = None;
    let mut expect_root_key = false;
    let mut pending_root_key_is_stream_options = false;
    let mut awaiting_stream_options_value = false;
    let mut in_string = false;
    let mut escaped = false;
    let mut string_is_root_key = false;
    let mut string_is_target_value = false;
    let mut root_key = Vec::with_capacity("stream_options".len());
    let mut active = None;
    let mut plan = None;
    let mut buffer = [0_u8; 1];

    while reader.read(&mut buffer)? != 0 {
        let byte = buffer[0];
        let current = position;
        position += 1;

        if in_string {
            if escaped {
                if string_is_root_key {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "escaped semantic key requires fail-open rewrite",
                    ));
                }
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'"' {
                in_string = false;
                if string_is_root_key {
                    pending_root_key_is_stream_options = root_key == target_key;
                    string_is_root_key = false;
                } else if string_is_target_value {
                    if let Some(ActiveStreamOptionsValue::String { start }) = active.take() {
                        plan = Some(IncludeUsageRewritePlan::ReplaceValue {
                            start,
                            end: current + 1,
                        });
                    }
                    string_is_target_value = false;
                }
                continue;
            }
            if string_is_root_key && root_key.len() <= "stream_options".len() {
                root_key.push(byte);
            }
            continue;
        }

        if awaiting_stream_options_value {
            if byte.is_ascii_whitespace() {
                continue;
            }
            awaiting_stream_options_value = false;
            active = Some(match byte {
                b'{' => ActiveStreamOptionsValue::Object {
                    start: current,
                    depth: depth + 1,
                    has_content: false,
                },
                b'[' => ActiveStreamOptionsValue::Composite {
                    start: current,
                    depth: depth + 1,
                },
                b'"' => {
                    in_string = true;
                    string_is_target_value = true;
                    ActiveStreamOptionsValue::String { start: current }
                }
                _ => ActiveStreamOptionsValue::Primitive {
                    start: current,
                    last_non_whitespace: current,
                },
            });
            if byte == b'"' {
                continue;
            }
        }

        if let Some(ActiveStreamOptionsValue::Primitive {
            start,
            last_non_whitespace,
        }) = active
        {
            if depth == 1 && matches!(byte, b',' | b'}') {
                plan = Some(IncludeUsageRewritePlan::ReplaceValue {
                    start,
                    end: last_non_whitespace + 1,
                });
                active = None;
            } else if !byte.is_ascii_whitespace() {
                active = Some(ActiveStreamOptionsValue::Primitive {
                    start,
                    last_non_whitespace: current,
                });
            }
        }

        if byte == b'"' {
            if let Some(ActiveStreamOptionsValue::Object {
                start,
                depth: target_depth,
                ..
            }) = active
                && depth >= target_depth
            {
                active = Some(ActiveStreamOptionsValue::Object {
                    start,
                    depth: target_depth,
                    has_content: true,
                });
            }
            in_string = true;
            string_is_root_key = depth == 1 && expect_root_key;
            if string_is_root_key {
                root_key.clear();
                expect_root_key = false;
            }
            continue;
        }

        if let Some(ActiveStreamOptionsValue::Object {
            start,
            depth: target_depth,
            has_content,
        }) = active
            && depth >= target_depth
            && !byte.is_ascii_whitespace()
            && !(byte == b'}' && depth == target_depth)
        {
            active = Some(ActiveStreamOptionsValue::Object {
                start,
                depth: target_depth,
                has_content: has_content || byte != b'{',
            });
        }

        match byte {
            b':' if depth == 1 => {
                awaiting_stream_options_value = pending_root_key_is_stream_options;
                pending_root_key_is_stream_options = false;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth == 1 {
                    expect_root_key = byte == b'{';
                }
            }
            b'}' | b']' => {
                match active {
                    Some(ActiveStreamOptionsValue::Object {
                        start,
                        depth: target_depth,
                        has_content,
                    }) if byte == b'}' && depth == target_depth => {
                        plan = Some(IncludeUsageRewritePlan::InsertObject {
                            start,
                            offset: current,
                            empty: !has_content,
                        });
                        active = None;
                    }
                    Some(ActiveStreamOptionsValue::Composite {
                        start,
                        depth: target_depth,
                    }) if depth == target_depth => {
                        plan = Some(IncludeUsageRewritePlan::ReplaceValue {
                            start,
                            end: current + 1,
                        });
                        active = None;
                    }
                    _ => {}
                }
                if depth == 0 {
                    return Ok(None);
                }
                depth -= 1;
                if depth == 0 && byte == b'}' {
                    root_close = Some(current);
                }
            }
            b',' if depth == 1 => expect_root_key = true,
            _ => {}
        }
    }

    Ok(plan.or_else(|| root_close.map(|offset| IncludeUsageRewritePlan::InsertRoot { offset })))
}

fn locate_include_usage_rewrite<R>(mut reader: R) -> io::Result<Option<IncludeUsageRewritePlan>>
where
    R: Read + Seek,
{
    let plan = locate_root_field_rewrite(&mut reader, b"stream_options")?;
    let Some(IncludeUsageRewritePlan::InsertObject {
        start,
        offset,
        empty,
    }) = plan
    else {
        return Ok(plan);
    };
    reader.seek(SeekFrom::Start(start as u64))?;
    let nested = locate_root_field_rewrite(
        (&mut reader).take((offset - start + 1) as u64),
        b"include_usage",
    )?;
    Ok(match nested {
        Some(IncludeUsageRewritePlan::ReplaceValue {
            start: nested_start,
            end,
        }) => Some(IncludeUsageRewritePlan::ReplaceIncludeUsageValue {
            start: start + nested_start,
            end: start + end,
        }),
        Some(IncludeUsageRewritePlan::InsertObject {
            start: nested_start,
            offset: nested_offset,
            ..
        }) => Some(IncludeUsageRewritePlan::ReplaceIncludeUsageValue {
            start: start + nested_start,
            end: start + nested_offset + 1,
        }),
        Some(IncludeUsageRewritePlan::ReplaceIncludeUsageValue { .. }) => {
            unreachable!("nested locator only emits generic replacement plans")
        }
        Some(IncludeUsageRewritePlan::InsertRoot { .. }) | None => {
            Some(IncludeUsageRewritePlan::InsertObject {
                start,
                offset,
                empty,
            })
        }
    })
}

fn include_usage_rewrite_segments(plan: IncludeUsageRewritePlan) -> (usize, usize, &'static [u8]) {
    match plan {
        IncludeUsageRewritePlan::InsertRoot { offset } => (offset, 0, INCLUDE_USAGE_ROOT_INSERTION),
        IncludeUsageRewritePlan::InsertObject { offset, empty, .. } => (
            offset,
            0,
            if empty {
                INCLUDE_USAGE_EMPTY_OBJECT_INSERTION
            } else {
                INCLUDE_USAGE_OBJECT_INSERTION
            },
        ),
        IncludeUsageRewritePlan::ReplaceValue { start, end } => {
            (start, end - start, INCLUDE_USAGE_REPLACEMENT)
        }
        IncludeUsageRewritePlan::ReplaceIncludeUsageValue { start, end } => {
            (start, end - start, b"true".as_slice())
        }
    }
}

fn copy_exact_bounded(
    reader: &mut std::fs::File,
    writer: &mut std::fs::File,
    mut bytes: usize,
) -> io::Result<()> {
    let mut buffer = [0_u8; INCLUDE_USAGE_COPY_BUFFER_BYTES];
    while bytes > 0 {
        let capacity = buffer.len();
        let read = reader.read(&mut buffer[..bytes.min(capacity)])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request snapshot ended during include_usage rewrite",
            ));
        }
        writer.write_all(&buffer[..read])?;
        bytes -= read;
    }
    Ok(())
}

fn copy_to_end_bounded(reader: &mut std::fs::File, writer: &mut std::fs::File) -> io::Result<()> {
    let mut buffer = [0_u8; INCLUDE_USAGE_COPY_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..read])?;
    }
}

async fn rewrite_snapshot_include_usage(
    proxy_request_id: u64,
    snapshot: &PoolReplayBodySnapshot,
) -> io::Result<Option<PoolReplayBodySnapshot>> {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(None),
        PoolReplayBodySnapshot::Memory(bytes) => {
            if bytes.len() > REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES {
                let temp_file = Arc::new(PoolReplayTempFile {
                    path: build_pool_replay_temp_path(proxy_request_id),
                });
                tokio::fs::write(&temp_file.path, bytes).await?;
                let file_snapshot = PoolReplayBodySnapshot::File {
                    temp_file,
                    size: bytes.len(),
                };
                return Box::pin(rewrite_snapshot_include_usage(
                    proxy_request_id,
                    &file_snapshot,
                ))
                .await;
            }
            let Some(plan) = locate_include_usage_rewrite(std::io::Cursor::new(bytes.as_ref()))?
            else {
                return Ok(None);
            };
            let (prefix_bytes, skipped_bytes, insertion) = include_usage_rewrite_segments(plan);
            let mut rewritten = Vec::with_capacity(
                bytes
                    .len()
                    .saturating_sub(skipped_bytes)
                    .saturating_add(insertion.len()),
            );
            rewritten.extend_from_slice(&bytes[..prefix_bytes]);
            rewritten.extend_from_slice(insertion);
            rewritten.extend_from_slice(&bytes[prefix_bytes + skipped_bytes..]);
            Ok(Some(
                pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(rewritten)).await?,
            ))
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let source = temp_file.path.clone();
            let destination = Arc::new(PoolReplayTempFile {
                path: build_pool_replay_temp_path(proxy_request_id),
            });
            let destination_for_worker = destination.clone();
            let source_size = *size;
            let rewritten_size =
                tokio::task::spawn_blocking(move || -> io::Result<Option<usize>> {
                    let plan = locate_include_usage_rewrite(std::fs::File::open(&source)?)?;
                    let Some(plan) = plan else {
                        return Ok(None);
                    };
                    let mut reader = std::fs::File::open(source)?;
                    let mut writer = std::fs::File::create(&destination_for_worker.path)?;
                    let (prefix_bytes, skipped_bytes, insertion) =
                        include_usage_rewrite_segments(plan);
                    copy_exact_bounded(&mut reader, &mut writer, prefix_bytes)?;
                    if skipped_bytes > 0 {
                        reader.seek(SeekFrom::Current(skipped_bytes as i64))?;
                    }
                    writer.write_all(insertion)?;
                    copy_to_end_bounded(&mut reader, &mut writer)?;
                    writer.flush()?;
                    Ok(Some(source_size - skipped_bytes + insertion.len()))
                })
                .await
                .map_err(|err| io::Error::other(err.to_string()))??;
            Ok(rewritten_size.map(|size| PoolReplayBodySnapshot::File {
                temp_file: destination,
                size,
            }))
        }
    }
}

/// Build the semantic projection once, reusing the replay snapshot for routing,
/// capture, and upstream preparation.
pub(crate) async fn project_request_semantics(
    proxy_request_id: u64,
    snapshot: PoolReplayBodySnapshot,
    target: ProxyCaptureTarget,
    auto_include_usage: bool,
) -> RequestSemanticProjection {
    let started = Instant::now();
    let body_len = pool_request_snapshot_body_bytes(&snapshot);
    if request_semantic_pipeline_mode() == RequestSemanticPipelineMode::Legacy {
        let Ok(original) = snapshot.to_bytes().await else {
            return RequestSemanticProjection {
                snapshot: snapshot.clone(),
                request_info: RequestCaptureInfo::default(),
                hosted_image_intent: ImageIntent::Unknown,
                upstream_snapshot: snapshot,
                request_body_for_capture: None,
                body_rewritten: false,
                parse_elapsed_ms: started.elapsed().as_millis() as u64,
                materialization_bytes: 0,
                buffer_bytes: 0,
                json_parse_count: 0,
                whole_body_materialization_count: 0,
                peak_business_buffer_bytes: 0,
                fallback_reason: Some("legacy_snapshot_materialization_failed"),
            };
        };
        let (upstream, request_info, body_rewritten, hosted_image_intent) =
            prepare_target_request_body_with_hosted_intent(
                target,
                original.to_vec(),
                auto_include_usage,
            );
        let json_parse_count = u8::from(!original.is_empty());
        let (upstream_snapshot, body_rewritten, fallback_reason) =
            match pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(upstream)).await {
                Ok(snapshot) => (snapshot, body_rewritten, None),
                Err(error) => {
                    warn!(
                        proxy_request_id,
                        error = %error,
                        fallback_reason = "rewritten_snapshot_persist_failed",
                        "request semantic projection kept the original snapshot"
                    );
                    (
                        snapshot.clone(),
                        false,
                        Some("rewritten_snapshot_persist_failed"),
                    )
                }
            };
        return RequestSemanticProjection {
            snapshot,
            request_info,
            hosted_image_intent,
            upstream_snapshot,
            request_body_for_capture: Some(original),
            body_rewritten,
            parse_elapsed_ms: started.elapsed().as_millis() as u64,
            materialization_bytes: body_len,
            buffer_bytes: body_len,
            json_parse_count,
            whole_body_materialization_count: u8::from(body_len > 0),
            peak_business_buffer_bytes: body_len,
            fallback_reason,
        };
    }
    let mut projected_json_parse_count = u8::from(body_len > 0);
    let mut semantic_parse_buffer_bytes = 0_usize;
    let mut fallback_reason = None;
    let (
        request_info,
        hosted_image_intent,
        upstream_snapshot,
        request_body_for_capture,
        body_rewritten,
        rewrite_buffer_bytes,
    ) = match &snapshot {
        PoolReplayBodySnapshot::Empty => (
            RequestCaptureInfo::default(),
            ImageIntent::Unknown,
            PoolReplayBodySnapshot::Empty,
            Some(Bytes::new()),
            false,
            0,
        ),
        PoolReplayBodySnapshot::Memory(bytes)
            if bytes.len() <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES =>
        {
            let (upstream, info, rewritten, hosted_image_intent) =
                prepare_target_request_body_with_hosted_intent(
                    target,
                    bytes.to_vec(),
                    auto_include_usage,
                );
            let capture = Some(bytes.clone());
            let (upstream_snapshot, body_rewritten) = if rewritten {
                match pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(upstream)).await
                {
                    Ok(snapshot) => (snapshot, true),
                    Err(error) => {
                        warn!(
                            proxy_request_id,
                            error = %error,
                            fallback_reason = "rewritten_snapshot_persist_failed",
                            "request semantic projection kept the original snapshot"
                        );
                        (snapshot.clone(), false)
                    }
                }
            } else {
                (snapshot.clone(), false)
            };
            (
                info,
                hosted_image_intent,
                upstream_snapshot,
                capture,
                body_rewritten,
                bytes.len(),
            )
        }
        _ => {
            let analysis = analyze_replay_snapshot_for_pool_routing(
                &snapshot,
                Some(target),
                proxy_request_id,
                "request_semantic_projection",
            )
            .await;
            projected_json_parse_count = analysis.json_parse_count;
            semantic_parse_buffer_bytes = REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES.min(body_len);
            let request_info = RequestCaptureInfo {
                model: analysis.requested_model,
                sticky_key: analysis.sticky_key,
                prompt_cache_key: analysis.prompt_cache_key,
                prompt_cache_key_attribution_source: None,
                contains_encrypted_content: analysis.contains_encrypted_content,
                image_intent: Some(analysis.image_intent.as_str().to_string()),
                requested_service_tier: analysis.requested_service_tier,
                reasoning_effort: analysis.reasoning_effort,
                compaction_request_kind: analysis.compaction_kind,
                is_stream: analysis.is_stream,
                parse_error: (analysis.parse_outcome != "parsed")
                    .then(|| format!("request_json_{}", analysis.parse_outcome)),
            };
            let should_rewrite =
                target.should_auto_include_usage() && auto_include_usage && request_info.is_stream;
            let rewritten_snapshot = if should_rewrite {
                match rewrite_snapshot_include_usage(proxy_request_id, &snapshot).await {
                    Ok(rewritten) => rewritten,
                    Err(error) => {
                        warn!(
                            proxy_request_id,
                            error = %error,
                            fallback_reason = "include_usage_rewrite_failed",
                            "request semantic projection kept the original snapshot"
                        );
                        fallback_reason = Some("include_usage_rewrite_failed");
                        None
                    }
                }
            } else {
                None
            };
            let body_rewritten = rewritten_snapshot.is_some();
            (
                request_info,
                analysis.hosted_image_intent,
                rewritten_snapshot.unwrap_or_else(|| snapshot.clone()),
                None,
                body_rewritten,
                if should_rewrite {
                    INCLUDE_USAGE_COPY_BUFFER_BYTES.min(body_len)
                } else {
                    0
                },
            )
        }
    };
    let buffer_bytes = rewrite_buffer_bytes.max(semantic_parse_buffer_bytes);

    let whole_body_materialization_count = u8::from(matches!(
        &snapshot,
        PoolReplayBodySnapshot::Memory(bytes)
            if !bytes.is_empty() && bytes.len() <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
    ));
    RequestSemanticProjection {
        snapshot,
        request_info,
        hosted_image_intent,
        upstream_snapshot,
        request_body_for_capture,
        body_rewritten,
        parse_elapsed_ms: started.elapsed().as_millis() as u64,
        materialization_bytes: if whole_body_materialization_count > 0 {
            body_len
        } else {
            0
        },
        buffer_bytes,
        json_parse_count: projected_json_parse_count,
        whole_body_materialization_count,
        peak_business_buffer_bytes: buffer_bytes,
        fallback_reason,
    }
}

pub(crate) fn build_pool_replay_temp_path(proxy_request_id: u64) -> PathBuf {
    let mut path = env::temp_dir();
    let unique_id = NEXT_POOL_REPLAY_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "cvm-pool-replay-{proxy_request_id}-{}-{unique_id}.bin",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_unsupported_model_from_route_error_supports_short_and_hyphenated_ids() {
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model o3 is not supported by this account",
            )
            .as_deref(),
            Some("o3")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported model: 'o4-mini'",
            )
            .as_deref(),
            Some("o4-mini")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model id `computer-use-preview` is not supported",
            )
            .as_deref(),
            Some("computer-use-preview")
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: pool upstream responded with 400: unsupported model: gpt-5.5",
            )
            .as_deref(),
            Some("gpt-5.5")
        );
    }

    #[test]
    fn extract_unsupported_model_from_route_error_ignores_non_model_bad_requests() {
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "request body is not supported for this endpoint",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::TOO_MANY_REQUESTS,
                "unsupported model: gpt-5.5",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "model is not supported",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "response_format is not supported for model gpt-4o",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: pool",
            ),
            None
        );
        assert_eq!(
            extract_unsupported_model_from_route_error(
                StatusCode::BAD_REQUEST,
                "unsupported_model: response_format is not supported for model gpt-4o",
            ),
            None
        );
    }

    #[test]
    fn classify_response_endpoint_capability_observation_is_conservative() {
        assert_eq!(
            classify_response_endpoint_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_response_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported endpoint: /v1/responses is not supported by this account"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_response_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported tool: image_generation is not supported by this account"),
            ),
            CapabilitySupport::Unknown
        );
    }

    #[test]
    fn classify_standalone_search_capability_observation_uses_route_specific_failures() {
        assert_eq!(
            classify_standalone_search_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(StatusCode::NOT_FOUND, None),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(
                StatusCode::METHOD_NOT_ALLOWED,
                Some("method not allowed"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported endpoint: /v1/alpha/search"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_standalone_search_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("request body is invalid"),
            ),
            CapabilitySupport::Unknown
        );
        for status in [
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
        ] {
            assert_eq!(
                classify_standalone_search_capability_observation(
                    status,
                    Some("standalone search request failed"),
                ),
                CapabilitySupport::Unknown
            );
        }
    }

    #[test]
    fn classify_response_image_tool_capability_observation_learns_tool_failures_only() {
        assert_eq!(
            classify_response_image_tool_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("unsupported tool: image_generation is not supported by this account"),
            ),
            CapabilitySupport::Unsupported
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some(
                    "Responses Lite rejected top-level tool type image_generation; use input.additional_tools",
                ),
            ),
            CapabilitySupport::Unknown
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("request body is invalid"),
            ),
            CapabilitySupport::Unknown
        );
        assert_eq!(
            classify_response_image_tool_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("invalid image size: width must be divisible by 64"),
            ),
            CapabilitySupport::Unknown
        );
    }

    #[test]
    fn image_tool_rewrite_audit_marks_lite_tools_as_client_owned() {
        let audit = image_tool_rewrite_audit(
            ProxyCaptureTarget::Responses,
            true,
            crate::ImageToolRewriteMode::ForceAdd,
        )
        .expect("Responses requests should have an image-tool rewrite audit");
        assert_eq!(audit["protocol"], "responses_lite");
        assert_eq!(audit["mode"], "force_add");
        assert_eq!(audit["outcome"], "skipped");
        assert_eq!(audit["reason"], "responses_lite_client_owned_tools");
    }

    #[test]
    fn codex_imagegen_full_force_add_uses_top_level_namespace_and_replaces_conflicts() {
        let mut request = serde_json::json!({
            "input": "draw a test image",
            "tools": [
                {"type": "image_generation"},
                {"type": "namespace", "name": "web", "tools": [{"type": "function", "name": "search"}]},
                {"type": "namespace", "name": "image_gen", "tools": [
                    {"type": "function", "name": "imagegen", "parameters": {"type": "object"}},
                    {"type": "function", "name": "imagegen", "parameters": {"type": "object", "properties": {"legacy": {"type": "string"}}}},
                    {"type": "function", "name": "keep_me"}
                ]},
                {"type": "namespace", "name": "image_gen", "tools": [
                    {"type": "function", "name": "imagegen", "parameters": {"type": "object", "properties": {"other": {"type": "boolean"}}}},
                    {"type": "function", "name": "also_keep_me"}
                ]}
            ],
            "tool_choice": {"type": "image_generation"}
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["outcome"], "replaced");
        assert_eq!(audit["hostedRemoved"], true);
        assert!(audit["existingSchemaFingerprint"].is_string());
        assert!(
            audit["schemaDiffPaths"]
                .as_array()
                .is_some_and(|paths| !paths.is_empty())
        );
        assert!(request.get("tool_choice").is_none());
        let tools = request["tools"].as_array().expect("top-level tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "web"));
        let imagegen = tools
            .iter()
            .filter(|tool| is_codex_imagegen_namespace(tool))
            .flat_map(|namespace| {
                namespace
                    .get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|tool| is_codex_imagegen_function(tool))
            .collect::<Vec<_>>();
        assert_eq!(imagegen, vec![&codex_imagegen_function()]);
        assert!(tools.iter().any(|tool| {
            tool.get("tools")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item["name"] == "keep_me"))
        }));
        assert!(tools.iter().any(|tool| {
            tool.get("tools")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item["name"] == "also_keep_me"))
        }));
    }

    #[test]
    fn codex_imagegen_lite_normalizes_input_and_preserves_other_tools() {
        let mut request = serde_json::json!({
            "input": "draw a test image",
            "tools": [{"type": "image_generation"}],
            "tool_choice": "image_generation",
            "reasoning": {"effort": "medium"},
            "parallel_tool_calls": true
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["protocol"], "responses_lite");
        assert_eq!(audit["hostedRemoved"], true);
        assert_eq!(request["reasoning"]["context"], "all_turns");
        assert_eq!(request["parallel_tool_calls"], false);
        assert!(request.get("tool_choice").is_none());
        let input = request["input"].as_array().expect("normalized input array");
        assert_eq!(input[0]["type"], "additional_tools");
        assert_eq!(input[0]["role"], "developer");
        let tools = input[0]["tools"].as_array().expect("additional tools");
        assert_eq!(
            find_codex_imagegen_function(tools),
            Some(codex_imagegen_function())
        );
        assert_eq!(input[1]["content"], "draw a test image");
    }

    #[test]
    fn codex_imagegen_lite_migrates_top_level_namespaces_before_rewriting() {
        let web = serde_json::json!({
            "type": "namespace",
            "name": "web",
            "tools": [{"type": "function", "name": "search"}]
        });
        let legacy_imagegen = serde_json::json!({
            "type": "image_gen.imagegen",
            "version": "legacy"
        });
        let mut request = serde_json::json!({
            "input": [{"type": "message", "role": "user", "content": "draw"}],
            "tools": [
                {"type": "function", "name": "unrelated"},
                web.clone(),
                legacy_imagegen
            ]
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["outcome"], "replaced");
        assert_eq!(
            request["tools"],
            serde_json::json!([{"type": "function", "name": "unrelated"}])
        );
        let tools = request["input"][0]["tools"]
            .as_array()
            .expect("Lite developer tools");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(
            tools
                .iter()
                .all(|tool| tool["type"] != "image_gen.imagegen")
        );
        assert_eq!(
            find_codex_imagegen_function(tools),
            Some(codex_imagegen_function())
        );

        let (changed, _) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::Yes,
        );
        assert!(changed);
        let tools = request["input"][0]["tools"]
            .as_array()
            .expect("Lite developer tools after removal");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(find_codex_imagegen_function(tools).is_none());
    }

    #[test]
    fn codex_imagegen_lite_normalizes_object_additional_tools_and_removes_hosted_tools() {
        let web = serde_json::json!({
            "type": "namespace",
            "name": "web",
            "tools": [{"type": "function", "name": "search"}]
        });
        let mut request = serde_json::json!({
            "input": {
                "additional_tools": [
                    {"type": "image_generation"},
                    web.clone(),
                    {"type": "image_gen.imagegen", "version": "legacy"}
                ]
            }
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["hostedRemoved"], true);
        let input = request["input"].as_array().expect("normalized input array");
        let tools = input[0]["tools"].as_array().expect("Lite developer tools");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(tools.iter().all(|tool| tool["type"] != "image_generation"));
        assert!(
            tools
                .iter()
                .all(|tool| tool["type"] != "image_gen.imagegen")
        );
        assert_eq!(
            find_codex_imagegen_function(tools),
            Some(codex_imagegen_function())
        );

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::Yes,
        );
        assert!(changed);
        assert_eq!(audit["outcome"], "removed");
        let tools = request["input"][0]["tools"]
            .as_array()
            .expect("Lite developer tools after removal");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(find_codex_imagegen_function(tools).is_none());
    }

    #[test]
    fn codex_imagegen_lite_preserves_separate_developer_tool_entries() {
        let first_tool = serde_json::json!({
            "type": "namespace",
            "name": "web",
            "tools": [{"type": "function", "name": "search"}]
        });
        let second_tool = serde_json::json!({
            "type": "namespace",
            "name": "mcp",
            "tools": [{"type": "function", "name": "lookup"}]
        });
        let migrated_tool = serde_json::json!({
            "type": "namespace",
            "name": "computer",
            "tools": [{"type": "function", "name": "screenshot"}]
        });
        let mut request = serde_json::json!({
            "input": [
                {"type": "message", "role": "user", "content": "draw"},
                {"type": "additional_tools", "role": "developer", "tools": [first_tool.clone()]},
                {"type": "additional_tools", "role": "developer", "tools": [second_tool.clone()]}
            ],
            "tools": [migrated_tool.clone()]
        });

        let (changed, _) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        let input = request["input"].as_array().expect("Lite input array");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["content"], "draw");
        let first_tools = input[1]["tools"].as_array().expect("first developer tools");
        assert!(first_tools.iter().any(|tool| tool == &first_tool));
        assert!(first_tools.iter().any(|tool| tool == &migrated_tool));
        assert!(find_codex_imagegen_function(first_tools).is_some());
        assert_eq!(input[2]["tools"], serde_json::json!([second_tool]));
    }

    #[test]
    fn codex_imagegen_force_remove_without_a_tool_reports_no_change() {
        let mut request = serde_json::json!({"input": "summarize this"});

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::No,
        );

        assert!(!changed);
        assert_eq!(audit["outcome"], "no_change");
    }

    #[test]
    fn codex_imagegen_fill_missing_preserves_existing_schema_and_force_remove_keeps_other_tools() {
        let existing = serde_json::json!({
            "type": "namespace",
            "name": "image_gen",
            "tools": [{"type": "function", "name": "imagegen", "parameters": {"type": "object", "properties": {"legacy": {"type": "string"}}}}]
        });
        let mut request = serde_json::json!({
            "tools": [
                {"type": "namespace", "name": "web", "tools": [{"type": "function", "name": "search"}]},
                existing.clone()
            ]
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::FillMissing,
            crate::ImageIntent::Yes,
        );
        assert!(!changed);
        assert_eq!(audit["outcome"], "no_change");
        assert_eq!(request["tools"][1], existing);

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::Yes,
        );
        assert!(changed);
        assert_eq!(audit["outcome"], "removed");
        let tools = request["tools"].as_array().expect("tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "web"));
        assert!(find_codex_imagegen_function(tools).is_none());
    }

    #[test]
    fn codex_imagegen_keep_original_and_fill_missing_without_intent_do_not_inject() {
        let original = serde_json::json!({
            "input": "summarize this",
            "tools": [{"type": "image_generation"}],
            "tool_choice": {"type": "image_generation"}
        });
        let mut keep_original = original.clone();
        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut keep_original,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::KeepOriginal,
            crate::ImageIntent::Unknown,
        );
        assert!(!changed);
        assert_eq!(audit["outcome"], "no_change");
        assert_eq!(keep_original, original);

        let mut fill_missing = original;
        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut fill_missing,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::FillMissing,
            crate::ImageIntent::Unknown,
        );
        assert!(changed);
        assert_eq!(audit["outcome"], "skipped");
        assert_eq!(audit["hostedRemoved"], true);
        assert!(fill_missing["tools"].as_array().is_some_and(Vec::is_empty));
        assert!(fill_missing.get("tool_choice").is_none());
    }

    #[test]
    fn codex_imagegen_upstream_incompatibility_requires_a_canonical_namespace_and_known_502() {
        let injected = serde_json::json!({"outcome": "injected"});

        assert!(codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "Upstream request failed",
            Some(&injected),
        ));
        assert!(!codex_imagegen_upstream_incompatibility(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Upstream request failed",
            Some(&injected),
        ));
        assert!(!codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "Upstream request failed",
            Some(&serde_json::json!({"outcome": "no_change"})),
        ));
        let canonical_namespace_was_retained = serde_json::json!({
            "mode": "force_add",
            "outcome": "no_change",
            "reason": "already_current",
            "schemaDiffPaths": [],
        });
        assert!(codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "Upstream request failed",
            Some(&canonical_namespace_was_retained),
        ));
        assert!(!codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "temporary upstream timeout",
            Some(&injected),
        ));
    }

    #[tokio::test]
    async fn codex_imagegen_keep_original_records_audit_without_rewriting_snapshot() {
        let request = serde_json::json!({"input": "summarize this"});
        let request_bytes = serde_json::to_vec(&request).expect("serialize request fixture");
        let uri: Uri = "/v1/responses".parse().expect("responses uri");
        let snapshot = PoolReplayBodySnapshot::Memory(Bytes::from(request_bytes.clone()));

        let prepared = prepare_pool_request_body_for_account(
            79,
            Some(&snapshot),
            &uri,
            &Method::POST,
            Some("unsupported-encoding"),
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::KeepOriginal,
            Some(CodexImagegenProtocol::Full),
            None,
            None,
            None,
        )
        .await
        .expect("prepare keep-original Codex snapshot");

        assert!(!prepared.snapshot_is_decoded);
        assert_eq!(
            prepared
                .codex_imagegen_rewrite
                .as_ref()
                .and_then(|audit| audit.get("outcome"))
                .and_then(Value::as_str),
            Some("no_change")
        );
        assert_eq!(
            prepared
                .codex_imagegen_rewrite
                .as_ref()
                .and_then(|audit| audit.get("reason"))
                .and_then(Value::as_str),
            Some("policy_keep_original")
        );
        assert_eq!(
            prepared
                .snapshot
                .to_bytes()
                .await
                .expect("read original snapshot"),
            request_bytes
        );
    }

    #[tokio::test]
    async fn codex_imagegen_rewrites_gzip_and_file_replay_snapshots() {
        let request = serde_json::json!({
            "input": [{"type": "message", "role": "user", "content": "draw"}],
            "tools": [{"type": "namespace", "name": "web", "tools": []}]
        });
        let request_bytes = serde_json::to_vec(&request).expect("serialize request fixture");
        let mut gzip_encoder = GzEncoder::new(Vec::new(), Compression::default());
        gzip_encoder
            .write_all(&request_bytes)
            .expect("compress request fixture");
        let compressed = gzip_encoder
            .finish()
            .expect("finish compressed request fixture");
        let uri: Uri = "/v1/responses".parse().expect("responses uri");
        let compressed_snapshot = PoolReplayBodySnapshot::Memory(Bytes::from(compressed));

        let prepared = prepare_pool_request_body_for_account(
            77,
            Some(&compressed_snapshot),
            &uri,
            &Method::POST,
            Some("gzip"),
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::ForceAdd,
            Some(CodexImagegenProtocol::Full),
            None,
            None,
            None,
        )
        .await
        .expect("rewrite gzip snapshot");
        assert!(prepared.snapshot_is_decoded);
        assert_eq!(
            prepared
                .codex_imagegen_rewrite
                .as_ref()
                .and_then(|audit| audit.get("outcome"))
                .and_then(Value::as_str),
            Some("injected")
        );
        let prepared_json: Value = serde_json::from_slice(
            &prepared
                .snapshot
                .to_bytes()
                .await
                .expect("read rewritten gzip snapshot"),
        )
        .expect("decode rewritten gzip snapshot");
        assert!(
            find_codex_imagegen_function(
                prepared_json["tools"].as_array().expect("rewritten tools")
            )
            .is_some()
        );

        let file_path = build_pool_replay_temp_path(78);
        tokio::fs::write(&file_path, request_bytes)
            .await
            .expect("write file replay fixture");
        let file_snapshot = PoolReplayBodySnapshot::File {
            temp_file: Arc::new(PoolReplayTempFile { path: file_path }),
            size: request.to_string().len(),
        };
        let prepared = prepare_pool_request_body_for_account(
            78,
            Some(&file_snapshot),
            &uri,
            &Method::POST,
            None,
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::ForceAdd,
            Some(CodexImagegenProtocol::Full),
            None,
            None,
            None,
        )
        .await
        .expect("rewrite file replay snapshot");
        assert!(prepared.snapshot_is_decoded);
        let prepared_json: Value = serde_json::from_slice(
            &prepared
                .snapshot
                .to_bytes()
                .await
                .expect("read rewritten file snapshot"),
        )
        .expect("decode rewritten file snapshot");
        assert!(
            find_codex_imagegen_function(
                prepared_json["tools"].as_array().expect("rewritten tools")
            )
            .is_some()
        );
    }

    #[test]
    fn codex_imagegen_client_identification_does_not_guess_from_session_headers() {
        let mut lite_headers = HeaderMap::new();
        lite_headers.insert(
            "x-openai-internal-codex-responses-lite",
            HeaderValue::from_static("true"),
        );
        assert_eq!(
            codex_imagegen_protocol_from_headers(&lite_headers),
            Some(CodexImagegenProtocol::Lite)
        );

        let mut full_headers = HeaderMap::new();
        full_headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
        assert_eq!(
            codex_imagegen_protocol_from_headers(&full_headers),
            Some(CodexImagegenProtocol::Full)
        );

        let mut user_agent_headers = HeaderMap::new();
        user_agent_headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Codex Desktop/5.6"),
        );
        assert_eq!(
            codex_imagegen_protocol_from_headers(&user_agent_headers),
            Some(CodexImagegenProtocol::Full)
        );

        let mut unrelated_headers = HeaderMap::new();
        unrelated_headers.insert("session_id", HeaderValue::from_static("codex-desktop"));
        assert_eq!(
            codex_imagegen_protocol_from_headers(&unrelated_headers),
            None
        );
    }

    #[test]
    fn codex_imagegen_intent_does_not_require_hosted_image_capability() {
        let full = serde_json::json!({
            "tools": [{
                "type": "namespace",
                "name": "image_gen",
                "tools": [codex_imagegen_function()]
            }]
        });
        let lite = serde_json::json!({
            "input": [{
                "type": "additional_tools",
                "tools": [{
                    "type": "namespace",
                    "name": "image_gen",
                    "tools": [codex_imagegen_function()]
                }]
            }]
        });

        let lite_object = serde_json::json!({
            "input": {
                "additional_tools": [{"type": "image_gen.imagegen"}]
            }
        });

        for request in [&full, &lite, &lite_object] {
            assert_eq!(
                infer_image_intent_from_request_body(ProxyCaptureTarget::Responses, request),
                ImageIntent::Yes
            );
            assert_eq!(
                infer_hosted_image_intent_from_request_body(ProxyCaptureTarget::Responses, request),
                ImageIntent::No
            );
        }
    }

    #[test]
    fn pool_failover_error_retains_attempt_codex_imagegen_audit() {
        let mut last_error = None;
        let mut preserve_sticky_owner_terminal_error = false;
        let audit = serde_json::json!({"protocol": "responses_lite", "outcome": "injected"});

        store_pool_failover_error(
            &mut last_error,
            &mut preserve_sticky_owner_terminal_error,
            build_pool_no_available_account_error(0, 0, 0, None),
            Some(&audit),
        );

        assert_eq!(
            last_error
                .as_ref()
                .and_then(|error| error.codex_imagegen_rewrite.as_ref()),
            Some(&audit)
        );
    }

    #[test]
    fn classify_image_endpoint_capability_observation_learns_direct_image_failures() {
        assert_eq!(
            classify_image_endpoint_capability_observation(StatusCode::OK, None),
            CapabilitySupport::Supported
        );
        assert_eq!(
            classify_image_endpoint_capability_observation(
                StatusCode::BAD_REQUEST,
                Some("No available channel for model gpt-image-1 under group default"),
            ),
            CapabilitySupport::Unsupported
        );
    }

    #[tokio::test]
    async fn request_semantic_projection_rewrites_small_stream_body_without_changing_capture() {
        let original = Bytes::from_static(br#"{"model":"gpt-5","stream":true}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_001, original.clone())
            .await
            .expect("build replay snapshot");

        let projection =
            project_request_semantics(90_001, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(projection.body_rewritten);
        assert_eq!(projection.request_body_for_capture, Some(original));
        let rewritten = projection
            .upstream_snapshot
            .to_bytes()
            .await
            .expect("read rewritten body");
        let value: Value = serde_json::from_slice(&rewritten).expect("parse rewritten body");
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert!(projection.buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES);
    }

    #[tokio::test]
    async fn request_semantic_projection_rewrites_medium_memory_snapshot_via_file() {
        let prefix = br#"{"model":"gpt-5","stream":true,"stream_options":{"include_usage":false,"other_option":"preserved"},"padding":""#;
        let mut body = Vec::with_capacity(512 * 1024);
        body.extend_from_slice(prefix);
        body.resize(512 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_005, Bytes::from(body))
            .await
            .expect("build replay snapshot");
        assert!(matches!(&snapshot, PoolReplayBodySnapshot::Memory(_)));

        let projection =
            project_request_semantics(90_005, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(projection.body_rewritten);
        assert!(matches!(
            &projection.upstream_snapshot,
            PoolReplayBodySnapshot::File { .. }
        ));
        let forwarded = projection
            .upstream_snapshot
            .to_bytes()
            .await
            .expect("read medium rewritten body");
        assert_eq!(
            forwarded
                .windows(b"\"include_usage\"".len())
                .filter(|window| *window == b"\"include_usage\"")
                .count(),
            1
        );
        let value: Value = serde_json::from_slice(&forwarded).expect("parse medium rewritten body");
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert_eq!(value["stream_options"]["other_option"], "preserved");
        assert_eq!(projection.whole_body_materialization_count, 0);
        assert_eq!(projection.materialization_bytes, 0);
    }

    #[tokio::test]
    async fn request_semantic_projection_merges_existing_in_memory_stream_options() {
        let prefix = br#"{"model":"gpt-5","stream":true,"stream_options":{"include_usage":false,"other_option":"preserved"},"padding":""#;
        let mut body = Vec::with_capacity(128 * 1024);
        body.extend_from_slice(prefix);
        body.resize(128 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_008, Bytes::from(body))
            .await
            .expect("build replay snapshot");
        assert!(matches!(&snapshot, PoolReplayBodySnapshot::Memory(_)));

        let projection =
            project_request_semantics(90_008, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;
        let forwarded = projection
            .upstream_snapshot
            .to_bytes()
            .await
            .expect("read rewritten body");
        let value: Value = serde_json::from_slice(&forwarded).expect("parse rewritten body");

        assert!(projection.body_rewritten);
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert_eq!(value["stream_options"]["other_option"], "preserved");
        assert_eq!(
            forwarded
                .windows(b"\"stream_options\"".len())
                .filter(|window| *window == b"\"stream_options\"")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn request_semantic_projection_keeps_codex_image_intent_out_of_hosted_routing() {
        let prefix = br#"{"model":"gpt-5","input":[{"type":"additional_tools","tools":[{"type":"image_gen.imagegen"}]}],"padding":""#;
        let mut body = Vec::with_capacity(512 * 1024);
        body.extend_from_slice(prefix);
        body.resize(512 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_007, Bytes::from(body))
            .await
            .expect("build replay snapshot");
        let projection =
            project_request_semantics(90_007, snapshot, ProxyCaptureTarget::Responses, true).await;

        assert_eq!(projection.request_info.image_intent.as_deref(), Some("yes"));
        assert_eq!(projection.hosted_image_intent, ImageIntent::No);
        let prepared = prepare_pool_request_body_for_account(
            90_007,
            Some(&projection.upstream_snapshot),
            &"/v1/responses".parse().expect("valid responses uri"),
            &Method::POST,
            None,
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::KeepOriginal,
            None,
            Some(&projection.request_info),
            Some(projection.hosted_image_intent),
            None,
        )
        .await
        .expect("prepare projected Codex image request");
        assert_eq!(prepared.requested_image_intent, ImageIntent::Yes);
        assert_eq!(prepared.requested_hosted_image_intent, ImageIntent::No);
    }

    #[tokio::test]
    async fn request_semantic_projection_keeps_file_backed_bodies_file_backed() {
        for (request_id, size, target, case_suffix) in [
            (
                90_016,
                256 * 1024,
                ProxyCaptureTarget::ChatCompletions,
                "16",
            ),
            (90_064, 512 * 1024, ProxyCaptureTarget::Responses, "64"),
        ] {
            let prefix = match target {
                ProxyCaptureTarget::ChatCompletions => br#"{"model":"gpt-5","stream":true,"stream_options":{"include_usage":false,"other_option":"preserved"},"metadata":{"sticky_key":"sticky-16","prompt_cache_key":"cache-16"},"service_tier":"priority","reasoning_effort":"high","input":[{"encrypted_content":"cipher"}],"padding":""#.as_slice(),
                ProxyCaptureTarget::Responses => br#"{"model":"gpt-5","stream":true,"metadata":{"sticky_key":"sticky-64","prompt_cache_key":"cache-64"},"serviceTier":"priority","reasoning":{"effort":"medium"},"nested":{"type":"encrypted\u005fcontent"},"tools":[{"type":"image_generation"}],"context_management":[{"type":"compaction","compact_threshold":1000}],"padding":""#.as_slice(),
                _ => unreachable!(),
            };
            let suffix = br#""}"#;
            let mut body = Vec::with_capacity(size);
            body.extend_from_slice(prefix);
            body.resize(size - suffix.len(), b'x');
            body.extend_from_slice(suffix);
            let original_digest = Sha256::digest(&body);
            let snapshot = pool_replay_snapshot_from_bytes_with_memory_threshold(
                request_id,
                Bytes::from(body),
                64 * 1024,
            )
            .await
            .expect("build replay snapshot");
            assert!(matches!(&snapshot, PoolReplayBodySnapshot::File { .. }));

            let projection = project_request_semantics(request_id, snapshot, target, true).await;

            assert!(matches!(
                &projection.snapshot,
                PoolReplayBodySnapshot::File { .. }
            ));
            assert!(matches!(
                &projection.upstream_snapshot,
                PoolReplayBodySnapshot::File { .. }
            ));
            assert_eq!(projection.request_info.model.as_deref(), Some("gpt-5"));
            assert!(projection.request_info.is_stream);
            assert!(projection.request_info.contains_encrypted_content);
            assert_eq!(
                projection.request_info.requested_service_tier.as_deref(),
                Some("priority")
            );
            assert_eq!(
                projection.request_info.sticky_key.as_deref(),
                Some(format!("sticky-{case_suffix}").as_str())
            );
            assert_eq!(
                projection.request_info.prompt_cache_key.as_deref(),
                Some(format!("cache-{case_suffix}").as_str())
            );
            assert_eq!(projection.request_body_for_capture, None);
            assert_eq!(
                Sha256::digest(
                    projection
                        .snapshot
                        .to_bytes()
                        .await
                        .expect("read original replay snapshot")
                ),
                original_digest
            );
            if target == ProxyCaptureTarget::ChatCompletions {
                assert!(projection.body_rewritten);
                assert!(projection.buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES);
                assert_eq!(
                    projection.request_info.reasoning_effort.as_deref(),
                    Some("high")
                );
                let forwarded_bytes = projection
                    .upstream_snapshot
                    .to_bytes()
                    .await
                    .expect("read rewritten replay snapshot");
                assert_eq!(
                    forwarded_bytes
                        .windows(b"\"include_usage\"".len())
                        .filter(|window| *window == b"\"include_usage\"")
                        .count(),
                    1
                );
                let forwarded: Value = serde_json::from_slice(&forwarded_bytes)
                    .expect("parse rewritten replay snapshot");
                assert_eq!(forwarded["stream_options"]["include_usage"], true);
                assert_eq!(forwarded["stream_options"]["other_option"], "preserved");
                let PoolReplayBodySnapshot::File { temp_file, size } =
                    &projection.upstream_snapshot
                else {
                    panic!("rewritten body must remain file-backed");
                };
                let mut config = crate::tests::test_config();
                config.proxy_raw_dir = std::env::temp_dir().join(format!(
                    "cvm-request-semantic-raw-{}",
                    Utc::now().timestamp_nanos_opt().unwrap_or_default()
                ));
                config.proxy_raw_compression = RawCompressionCodec::None;
                config.proxy_raw_max_bytes = None;
                let raw = store_raw_payload_snapshot_file(
                    &config,
                    "request-semantic-file-backed",
                    "request",
                    temp_file.path.clone(),
                    *size,
                )
                .await;
                let raw_path = raw.path.expect("raw capture path");
                assert_eq!(
                    Sha256::digest(tokio::fs::read(&raw_path).await.expect("read raw capture")),
                    Sha256::digest(&forwarded_bytes)
                );
                let _ = tokio::fs::remove_dir_all(&config.proxy_raw_dir).await;
            } else {
                assert!(!projection.body_rewritten);
                assert!(projection.buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES);
                assert_eq!(
                    projection.request_info.reasoning_effort.as_deref(),
                    Some("medium")
                );
                assert_eq!(projection.request_info.image_intent.as_deref(), Some("yes"));
                assert_eq!(
                    projection.request_info.compaction_request_kind,
                    Some(CompactionKind::RemoteV2)
                );
                assert_eq!(
                    Sha256::digest(
                        projection
                            .upstream_snapshot
                            .to_bytes()
                            .await
                            .expect("read forwarded replay snapshot")
                    ),
                    original_digest
                );
                for _ in 0..2 {
                    let replay = counted_http_body_from_snapshot(
                        &projection.upstream_snapshot,
                        ObservedByteCounter::default(),
                    );
                    assert_eq!(
                        Sha256::digest(
                            axum::body::to_bytes(replay, size + 1)
                                .await
                                .expect("read direct retry replay body")
                        ),
                        original_digest
                    );
                }
            }
            assert_eq!(projection.json_parse_count, 1);
            assert_eq!(projection.whole_body_materialization_count, 0);
            assert_eq!(projection.materialization_bytes, 0);
            assert!(
                projection.peak_business_buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
            );
        }
    }

    #[tokio::test]
    async fn request_semantic_projection_fails_open_for_invalid_large_json() {
        let original = Bytes::from(vec![b'!'; 2 * 1024 * 1024]);
        let snapshot = pool_replay_snapshot_from_bytes(90_002, original.clone())
            .await
            .expect("build replay snapshot");
        let projection =
            project_request_semantics(90_002, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(!projection.body_rewritten);
        assert!(projection.request_info.parse_error.is_some());
        assert_eq!(
            projection
                .upstream_snapshot
                .to_bytes()
                .await
                .expect("read fail-open body"),
            original
        );
        assert_eq!(projection.whole_body_materialization_count, 0);
        assert_eq!(projection.materialization_bytes, 0);
    }

    #[tokio::test]
    async fn request_semantic_projection_bounds_large_escaped_selected_string() {
        for (request_id, size) in [(90_003, 512 * 1024), (90_004, 2 * 1024 * 1024)] {
            let mut body = Vec::with_capacity(size);
            body.extend_from_slice(br#"{"model":""#);
            while body.len() < size {
                body.extend_from_slice(br#"\u0061"#);
            }
            body.extend_from_slice(br#"","stream":true}"#);
            let original = Bytes::from(body);
            let snapshot = pool_replay_snapshot_from_bytes(request_id, original.clone())
                .await
                .expect("build replay snapshot");

            let projection = project_request_semantics(
                request_id,
                snapshot,
                ProxyCaptureTarget::ChatCompletions,
                true,
            )
            .await;

            assert!(projection.request_info.parse_error.is_some());
            assert!(!projection.body_rewritten);
            assert_eq!(
                projection
                    .upstream_snapshot
                    .to_bytes()
                    .await
                    .expect("read bounded fail-open body"),
                original
            );
            assert_eq!(projection.whole_body_materialization_count, 0);
            assert_eq!(projection.materialization_bytes, 0);
            assert!(
                projection.peak_business_buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
            );
        }
    }

    #[tokio::test]
    async fn request_semantic_projection_fails_open_for_escaped_rewrite_key() {
        let prefix =
            br#"{"model":"gpt-5","stream":true,"stream\u005foptions":{"include_usage":false},"padding":""#;
        let mut body = Vec::with_capacity(512 * 1024);
        body.extend_from_slice(prefix);
        body.resize(512 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let original = Bytes::from(body);
        let snapshot = pool_replay_snapshot_from_bytes(90_006, original.clone())
            .await
            .expect("build replay snapshot");

        let projection =
            project_request_semantics(90_006, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(!projection.body_rewritten);
        assert_eq!(
            projection
                .upstream_snapshot
                .to_bytes()
                .await
                .expect("read escaped-key fail-open body"),
            original
        );
        assert_eq!(projection.whole_body_materialization_count, 0);
    }
}
