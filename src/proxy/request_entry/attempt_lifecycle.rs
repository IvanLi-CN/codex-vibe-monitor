use super::*;

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

#[derive(Debug, Clone, Default)]
pub(crate) struct PendingPoolAttemptRecord {
    pub(crate) attempt_id: Option<i64>,
    pub(crate) attempt_public_id: Option<String>,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) endpoint: String,
    pub(crate) sticky_key: Option<String>,
    pub(crate) routing_source: Option<String>,
    pub(crate) routing_selection_audit_json: Option<String>,
    pub(crate) sticky_affinity_generation: Option<i64>,
    pub(crate) requester_ip: Option<String>,
    pub(crate) upstream_base_url_host: Option<String>,
    pub(crate) group_name_snapshot: Option<String>,
    pub(crate) proxy_binding_key_snapshot: Option<String>,
    pub(crate) request_model: Option<String>,
    pub(crate) upstream_request_model: Option<String>,
    pub(crate) model_mapping_pattern: Option<String>,
    pub(crate) request_summary_json: Option<String>,
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
pub(crate) const POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_INTERRUPTED: &str = "interrupted";

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
    prompt_cache_key: Option<String>,
    pending_prompt_cache_key_releases: Vec<String>,
}

impl PoolViaRuntimeSnapshotCleanupGuard {
    pub(crate) fn new(state: Arc<AppState>, invoke_id: impl Into<String>) -> Self {
        Self {
            state,
            invoke_id: invoke_id.into(),
            prompt_cache_key: None,
            pending_prompt_cache_key_releases: Vec::new(),
        }
    }

    pub(crate) fn set_invoke_id(&mut self, invoke_id: impl Into<String>) {
        self.invoke_id = invoke_id.into();
    }

    pub(crate) async fn adopt_prompt_cache_key(&mut self, prompt_cache_key: Option<&str>) {
        let next_prompt_cache_key = prompt_cache_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        if self.prompt_cache_key == next_prompt_cache_key {
            return;
        }
        let previous_prompt_cache_key =
            std::mem::replace(&mut self.prompt_cache_key, next_prompt_cache_key);
        if let Some(previous_prompt_cache_key) = previous_prompt_cache_key {
            self.pending_prompt_cache_key_releases
                .push(previous_prompt_cache_key.clone());
            release_active_prompt_cache_conversation(
                &self.state.prompt_cache_conversation_cache,
                &previous_prompt_cache_key,
            )
            .await;
            self.pending_prompt_cache_key_releases
                .retain(|key| key != &previous_prompt_cache_key);
        }
    }
}

impl Drop for PoolViaRuntimeSnapshotCleanupGuard {
    fn drop(&mut self) {
        let mut prompt_cache_keys = std::mem::take(&mut self.pending_prompt_cache_key_releases);
        if let Some(prompt_cache_key) = self.prompt_cache_key.take() {
            prompt_cache_keys.push(prompt_cache_key);
        }
        if !prompt_cache_keys.is_empty() {
            let cache = self.state.prompt_cache_conversation_cache.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    release_active_prompt_cache_conversations(&cache, &prompt_cache_keys).await;
                });
            }
        }
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
        crate::observability::diagnostics::spawn(async move {
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

pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS: u64 = 10;
pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS: u64 = 250;
pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS: u64 = 10;
pub(crate) const POOL_NO_AVAILABLE_ACCOUNT_MESSAGE: &str = "no healthy pool account is available";

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolNoAvailableWaitSettings {
    pub(crate) timeout: Duration,
    pub(crate) poll_interval: Duration,
    pub(crate) retry_after_secs: u64,
}

impl Default for PoolNoAvailableWaitSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS),
            poll_interval: Duration::from_millis(
                DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS,
            ),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        }
    }
}

impl PoolNoAvailableWaitSettings {
    pub(crate) fn normalized_poll_interval(self) -> Duration {
        if self.poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            self.poll_interval
        }
    }
}

#[derive(Debug)]
pub(crate) enum PoolAccountResolutionWithWait {
    Resolution(PoolAccountResolution),
    TotalTimeoutExpired,
}

pub(crate) const POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS: u8 = 3;

#[cfg(test)]
mod tests {
    use super::*;

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
}
