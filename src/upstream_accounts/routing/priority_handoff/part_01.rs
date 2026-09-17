use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(crate) const PRIORITY_HANDOFF_ROUTING_SOURCE: &str = "priorityHandoff";
pub(crate) const PRIORITY_HANDOFF_SUCCEEDED_REASON: &str = "priorityHandoffSucceeded";
pub(crate) const PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON: &str = "priorityHandoffFailureCooldown";
pub(crate) const PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON: &str =
    "priorityHandoffRecoveryProgress";
const PRIORITY_HANDOFF_VERIFICATION_SUCCESSES: u8 = 3;
const PRIORITY_HANDOFF_FIRST_COOLDOWN_SECS: u64 =
    super::model_health::MODEL_ROUTE_COOLDOWN_BASE_SECS as u64;
const PRIORITY_HANDOFF_MAX_COOLDOWN_SECS: u64 =
    super::model_health::MODEL_ROUTE_COOLDOWN_MAX_SECS as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PriorityHandoffPhase {
    Verifying,
    Open,
    CoolingDown,
}
#[derive(Debug)]
struct PriorityHandoffEntry {
    epoch: u64,
    generation: u64,
    phase: PriorityHandoffPhase,
    verification_successes: u8,
    failure_streak: u32,
    cooldown_until: Option<Instant>,
    in_flight: bool,
    in_flight_generation: Option<u64>,
    pending_failure_cooldown: bool,
}

#[derive(Debug, Default)]
struct PriorityHandoffState {
    enabled: bool,
    generation: u64,
    next_generation: u64,
    entries: HashMap<(i64, String), PriorityHandoffEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct PriorityHandoffAttemptContext {
    pub(crate) account_id: i64,
    pub(crate) model_key: String,
    pub(crate) generation: u64,
}

impl PriorityHandoffState {
    fn new() -> Self {
        Self {
            enabled: true,
            generation: 1,
            next_generation: 1,
            entries: HashMap::new(),
        }
    }
}

fn allocate_generation(state: &mut PriorityHandoffState) -> u64 {
    state.next_generation = state
        .next_generation
        .max(state.generation)
        .saturating_add(1);
    state.next_generation
}

fn state() -> &'static Mutex<PriorityHandoffState> {
    static STATE: OnceLock<Mutex<PriorityHandoffState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PriorityHandoffState::new()))
}

#[cfg(test)]
fn priority_handoff_test_lock() -> &'static Arc<tokio::sync::Mutex<()>> {
    static TEST_LOCK: OnceLock<Arc<tokio::sync::Mutex<()>>> = OnceLock::new();
    TEST_LOCK.get_or_init(|| Arc::new(tokio::sync::Mutex::new(())))
}

#[cfg(test)]
pub(crate) async fn priority_handoff_test_guard() -> tokio::sync::OwnedMutexGuard<()> {
    priority_handoff_test_lock().clone().lock_owned().await
}

#[cfg(test)]
pub(crate) fn priority_handoff_test_guard_blocking() -> tokio::sync::OwnedMutexGuard<()> {
    priority_handoff_test_lock().clone().blocking_lock_owned()
}

fn attempt_contexts() -> &'static Mutex<HashMap<i64, PriorityHandoffAttemptContext>> {
    static ATTEMPT_CONTEXTS: OnceLock<Mutex<HashMap<i64, PriorityHandoffAttemptContext>>> =
        OnceLock::new();
    ATTEMPT_CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn invoke_attempt_contexts() -> &'static Mutex<HashMap<String, PriorityHandoffAttemptContext>> {
    static INVOKE_ATTEMPT_CONTEXTS: OnceLock<
        Mutex<HashMap<String, PriorityHandoffAttemptContext>>,
    > = OnceLock::new();
    INVOKE_ATTEMPT_CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_model_key(model: Option<&str>) -> Option<String> {
    model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn priority_handoff_generation_from_audit_json(audit_json: Option<&str>) -> Option<u64> {
    audit_json
        .and_then(|json| serde_json::from_str::<PoolRoutingSelectionAudit>(json).ok())
        .and_then(|audit| {
            audit
                .handoff_admission
                .map(|admission| admission.generation)
        })
}

pub(crate) fn remember_priority_handoff_attempt(
    attempt_id: Option<i64>,
    invoke_id: Option<&str>,
    account_id: i64,
    requested_model: Option<&str>,
    audit_json: Option<&str>,
) {
    let (Some(model_key), Some(generation)) = (
        normalize_model_key(requested_model),
        priority_handoff_generation_from_audit_json(audit_json),
    ) else {
        return;
    };
    let context = PriorityHandoffAttemptContext {
        account_id,
        model_key,
        generation,
    };
    if let Some(attempt_id) = attempt_id
        && let Ok(mut contexts) = attempt_contexts().lock()
    {
        contexts.insert(attempt_id, context.clone());
    }
    if attempt_id.is_none()
        && let Some(invoke_id) = invoke_id.filter(|value| !value.is_empty())
        && let Ok(mut contexts) = invoke_attempt_contexts().lock()
    {
        contexts.insert(invoke_id.to_string(), context);
    }
}

pub(crate) fn take_priority_handoff_attempt(
    attempt_id: i64,
) -> Option<PriorityHandoffAttemptContext> {
    attempt_contexts()
        .lock()
        .ok()
        .and_then(|mut contexts| contexts.remove(&attempt_id))
}

fn priority_handoff_attempt_context(attempt_id: i64) -> Option<PriorityHandoffAttemptContext> {
    attempt_contexts()
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(&attempt_id).cloned())
}

fn priority_handoff_invoke_context(invoke_id: &str) -> Option<PriorityHandoffAttemptContext> {
    invoke_attempt_contexts()
        .lock()
        .ok()
        .and_then(|contexts| contexts.get(invoke_id).cloned())
}

fn take_priority_handoff_attempt_for_invoke(
    invoke_id: &str,
) -> Option<PriorityHandoffAttemptContext> {
    invoke_attempt_contexts()
        .lock()
        .ok()
        .and_then(|mut contexts| contexts.remove(invoke_id))
}

pub(crate) fn forget_priority_handoff_attempt(attempt_id: Option<i64>) {
    if let Some(attempt_id) = attempt_id {
        let _ = take_priority_handoff_attempt(attempt_id);
    }
}

pub(crate) fn forget_priority_handoff_attempt_for_invoke(invoke_id: &str) {
    let _ = take_priority_handoff_attempt_for_invoke(invoke_id);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PriorityHandoffAdmissionDecision {
    Disabled,
    PermitBusy,
    CoolingDown,
    Open,
    Admitted { generation: u64 },
}

#[derive(Debug)]
pub(crate) struct PriorityHandoffPermit {
    account_id: i64,
    model_key: String,
    generation: u64,
    completed: AtomicBool,
    sticky_migration: AtomicBool,
}

impl PriorityHandoffPermit {
    pub(crate) fn mark_sticky_migration(&self) {
        self.sticky_migration.store(true, Ordering::Release);
    }

    pub(crate) fn is_sticky_migration(&self) -> bool {
        self.sticky_migration.load(Ordering::Acquire)
    }

    pub(crate) fn complete_success(&self) -> Option<&'static str> {
        if self.completed.swap(true, Ordering::AcqRel) {
            return None;
        }
        complete_success_for_key(self.account_id, &self.model_key, self.generation)
    }

    pub(crate) fn complete_failure(&self, cooldown: bool) -> Option<&'static str> {
        if self.completed.swap(true, Ordering::AcqRel) {
            return None;
        }
        complete_failure_for_key(self.account_id, &self.model_key, self.generation, cooldown)
    }
}

impl Drop for PriorityHandoffPermit {
    fn drop(&mut self) {
        if !self.completed.load(Ordering::Acquire) {
            release_priority_handoff_for_key(self.account_id, &self.model_key, self.generation);
        }
    }
}

pub(crate) fn priority_handoff_admission_enabled() -> bool {
    state().lock().map(|state| state.enabled).unwrap_or(true)
}

pub(crate) fn set_priority_handoff_admission_enabled(enabled: bool) {
    let Ok(mut state) = state().lock() else {
        return;
    };
    if state.enabled == enabled {
        return;
    }
    state.enabled = enabled;
    state.generation = allocate_generation(&mut state);
    let generation = state.generation;
    for entry in state.entries.values_mut() {
        entry.epoch = generation;
    }
    if enabled {
        for entry in state.entries.values_mut() {
            entry.phase = PriorityHandoffPhase::Verifying;
            entry.verification_successes = 0;
            entry.failure_streak = 0;
            entry.cooldown_until = None;
            entry.generation = generation;
            entry.pending_failure_cooldown = false;
        }
    }
}

pub(crate) fn restart_priority_handoff_verification_for_model(
    account_id: i64,
    requested_model: &str,
) {
    let Some(model_key) = normalize_model_key(Some(requested_model)) else {
        return;
    };
    let Ok(mut state) = state().lock() else {
        return;
    };
    let generation = allocate_generation(&mut state);
    let epoch = state.generation;
    let entry = state
        .entries
        .entry((account_id, model_key))
        .or_insert_with(|| PriorityHandoffEntry {
            epoch,
            generation,
            phase: PriorityHandoffPhase::Verifying,
            verification_successes: 0,
            failure_streak: 0,
            cooldown_until: None,
            in_flight: false,
            in_flight_generation: None,
            pending_failure_cooldown: false,
        });
    entry.generation = generation;
    entry.epoch = epoch;
    entry.phase = PriorityHandoffPhase::Verifying;
    entry.verification_successes = 0;
    entry.failure_streak = 0;
    entry.cooldown_until = None;
    entry.pending_failure_cooldown = false;
}

pub(crate) fn reset_priority_handoff_for_model(account_id: i64, requested_model: &str) {
    restart_priority_handoff_verification_for_model(account_id, requested_model);
}

pub(crate) fn admit_priority_handoff(
    account_id: i64,
    requested_model: Option<&str>,
) -> (
    PriorityHandoffAdmissionDecision,
    Option<Arc<PriorityHandoffPermit>>,
) {
    let Some(model_key) = normalize_model_key(requested_model) else {
        return (PriorityHandoffAdmissionDecision::Open, None);
    };
    let Ok(mut state) = state().lock() else {
        return (PriorityHandoffAdmissionDecision::PermitBusy, None);
    };
    if !state.enabled {
        return (PriorityHandoffAdmissionDecision::Disabled, None);
    }
    let global_generation = state.generation;
    // Every admitted attempt gets a unique generation. Besides fencing global
    // and manual resets, this keeps a late Drop or callback from touching a
    // later permit for the same account/model pair.
    let admission_generation = allocate_generation(&mut state);
    let entry = state
        .entries
        .entry((account_id, model_key.clone()))
        .or_insert_with(|| PriorityHandoffEntry {
            epoch: global_generation,
            generation: global_generation,
            phase: PriorityHandoffPhase::Verifying,
            verification_successes: 0,
            failure_streak: 0,
            cooldown_until: None,
            in_flight: false,
            in_flight_generation: None,
            pending_failure_cooldown: false,
        });
    if entry.epoch != global_generation {
        entry.epoch = global_generation;
        entry.generation = global_generation;
        entry.phase = PriorityHandoffPhase::Verifying;
        entry.verification_successes = 0;
        entry.failure_streak = 0;
        entry.cooldown_until = None;
        entry.in_flight = false;
        entry.in_flight_generation = None;
        entry.pending_failure_cooldown = false;
    }
    if entry.in_flight {
        return (PriorityHandoffAdmissionDecision::PermitBusy, None);
    }
    if let Some(until) = entry.cooldown_until {
        if until > Instant::now() {
            entry.phase = PriorityHandoffPhase::CoolingDown;
            return (PriorityHandoffAdmissionDecision::CoolingDown, None);
        }
        entry.cooldown_until = None;
        entry.phase = PriorityHandoffPhase::Verifying;
    }
    if entry.phase == PriorityHandoffPhase::Open {
        return (PriorityHandoffAdmissionDecision::Open, None);
    }
    entry.in_flight = true;
    entry.generation = admission_generation;
    entry.in_flight_generation = Some(admission_generation);
    let generation = admission_generation;
    (
        PriorityHandoffAdmissionDecision::Admitted { generation },
        Some(Arc::new(PriorityHandoffPermit {
            account_id,
            model_key,
            generation,
            completed: AtomicBool::new(false),
            sticky_migration: AtomicBool::new(false),
        })),
    )
}

pub(crate) fn priority_handoff_admission_snapshot(
    account_id: i64,
    requested_model: Option<&str>,
) -> (String, u8) {
    let Some(model_key) = normalize_model_key(requested_model) else {
        return ("open".to_string(), PRIORITY_HANDOFF_VERIFICATION_SUCCESSES);
    };
    let Ok(mut state) = state().lock() else {
        return ("verifying".to_string(), 0);
    };
    let generation = state.generation;
    let entry = state
        .entries
        .entry((account_id, model_key))
        .or_insert_with(|| PriorityHandoffEntry {
            epoch: generation,
            generation,
            phase: PriorityHandoffPhase::Verifying,
            verification_successes: 0,
            failure_streak: 0,
            cooldown_until: None,
            in_flight: false,
            in_flight_generation: None,
            pending_failure_cooldown: false,
        });
    if entry.epoch != generation {
        entry.epoch = generation;
        entry.generation = generation;
        entry.phase = PriorityHandoffPhase::Verifying;
        entry.verification_successes = 0;
        entry.failure_streak = 0;
        entry.cooldown_until = None;
        entry.in_flight = false;
        entry.in_flight_generation = None;
        entry.pending_failure_cooldown = false;
    }
    if entry
        .cooldown_until
        .is_some_and(|until| until <= Instant::now())
    {
        entry.cooldown_until = None;
        entry.phase = PriorityHandoffPhase::Verifying;
    }
    (
        match entry.phase {
            PriorityHandoffPhase::Verifying => "verifying",
            PriorityHandoffPhase::Open => "open",
            PriorityHandoffPhase::CoolingDown => "coolingDown",
        }
        .to_string(),
        entry.verification_successes,
    )
}

pub(crate) fn priority_handoff_generation(
    account_id: i64,
    requested_model: Option<&str>,
) -> Option<u64> {
    let model_key = normalize_model_key(requested_model)?;
    state().lock().ok().map(|state| {
        state
            .entries
            .get(&(account_id, model_key))
            .map(|entry| entry.generation)
            .unwrap_or(state.generation)
    })
}

pub(crate) fn priority_handoff_client_cancellation(
    status: &str,
    downstream_http_status: Option<StatusCode>,
    failure_kind: Option<&str>,
) -> bool {
    status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
        && downstream_http_status.is_some()
        && failure_kind.is_none()
}

pub(crate) fn complete_priority_handoff_for_request(
    account_id: i64,
    requested_model: Option<&str>,
    generation: Option<u64>,
    success: bool,
    cooldown: bool,
) -> Option<&'static str> {
    let model_key = normalize_model_key(requested_model)?;
    let Ok(mut state) = state().lock() else {
        return None;
    };
    let entry = state.entries.get_mut(&(account_id, model_key))?;
    if generation.is_some_and(|generation| generation != entry.generation) {
        if generation == entry.in_flight_generation {
            entry.in_flight = false;
            entry.in_flight_generation = None;
            entry.pending_failure_cooldown = false;
        }
        return None;
    }
    if !entry.in_flight {
        return None;
    }
    entry.in_flight = false;
    entry.in_flight_generation = None;
    entry.pending_failure_cooldown = false;
    if success {
        entry.failure_streak = 0;
        entry.cooldown_until = None;
        entry.verification_successes = entry
            .verification_successes
            .saturating_add(1)
            .min(PRIORITY_HANDOFF_VERIFICATION_SUCCESSES);
        entry.phase = if entry.verification_successes >= PRIORITY_HANDOFF_VERIFICATION_SUCCESSES {
            PriorityHandoffPhase::Open
        } else {
            PriorityHandoffPhase::Verifying
        };
        Some(if entry.phase == PriorityHandoffPhase::Open {
            PRIORITY_HANDOFF_SUCCEEDED_REASON
        } else {
            PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON
        })
    } else if cooldown {
        entry.failure_streak = entry.failure_streak.saturating_add(1);
        let shift = entry.failure_streak.saturating_sub(1).min(8);
        let cooldown_secs = PRIORITY_HANDOFF_FIRST_COOLDOWN_SECS
            .saturating_mul(1_u64 << shift)
            .min(PRIORITY_HANDOFF_MAX_COOLDOWN_SECS);
        entry.cooldown_until = Some(Instant::now() + Duration::from_secs(cooldown_secs));
        entry.phase = PriorityHandoffPhase::CoolingDown;
        entry.verification_successes = 0;
        Some(PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON)
    } else {
        entry.phase = PriorityHandoffPhase::Verifying;
        None
    }
}

fn complete_success_for_key(
    account_id: i64,
    model_key: &str,
    generation: u64,
) -> Option<&'static str> {
    let Ok(mut state) = state().lock() else {
        return None;
    };
    let entry = state
        .entries
        .get_mut(&(account_id, model_key.to_string()))?;
    if !entry.in_flight {
        return None;
    }
    if entry.generation != generation {
        if entry.in_flight_generation == Some(generation) {
            entry.in_flight = false;
            entry.in_flight_generation = None;
            entry.pending_failure_cooldown = false;
        }
        return None;
    }
    entry.in_flight = false;
    entry.in_flight_generation = None;
    entry.pending_failure_cooldown = false;
    entry.failure_streak = 0;
    entry.cooldown_until = None;
    entry.verification_successes = entry
        .verification_successes
        .saturating_add(1)
        .min(PRIORITY_HANDOFF_VERIFICATION_SUCCESSES);
    entry.phase = if entry.verification_successes >= PRIORITY_HANDOFF_VERIFICATION_SUCCESSES {
        PriorityHandoffPhase::Open
    } else {
        PriorityHandoffPhase::Verifying
    };
    Some(if entry.phase == PriorityHandoffPhase::Open {
        PRIORITY_HANDOFF_SUCCEEDED_REASON
    } else {
        PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON
    })
}

fn complete_failure_for_key(
    account_id: i64,
    model_key: &str,
    generation: u64,
    cooldown: bool,
) -> Option<&'static str> {
    let Ok(mut state) = state().lock() else {
        return None;
    };
    let entry = state
        .entries
        .get_mut(&(account_id, model_key.to_string()))?;
    if !entry.in_flight {
        return None;
    }
    if entry.generation != generation {
        if entry.in_flight_generation == Some(generation) {
            entry.in_flight = false;
            entry.in_flight_generation = None;
            entry.pending_failure_cooldown = false;
        }
        return None;
    }
    entry.in_flight = false;
    entry.in_flight_generation = None;
    entry.pending_failure_cooldown = false;
    if !cooldown {
        entry.phase = PriorityHandoffPhase::Verifying;
        return None;
    }
    Some(apply_failure_cooldown(entry))
}

fn apply_failure_cooldown(entry: &mut PriorityHandoffEntry) -> &'static str {
    entry.failure_streak = entry.failure_streak.saturating_add(1);
    let shift = entry.failure_streak.saturating_sub(1).min(8);
    let cooldown_secs = PRIORITY_HANDOFF_FIRST_COOLDOWN_SECS
        .saturating_mul(1_u64 << shift)
        .min(PRIORITY_HANDOFF_MAX_COOLDOWN_SECS);
    entry.cooldown_until = Some(Instant::now() + Duration::from_secs(cooldown_secs));
    entry.phase = PriorityHandoffPhase::CoolingDown;
    entry.verification_successes = 0;
    PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON
}

pub(crate) fn defer_priority_handoff_failure_for_key(
    account_id: i64,
    model_key: &str,
    generation: u64,
    cooldown: bool,
) -> bool {
    let Ok(mut state) = state().lock() else {
        return false;
    };
    let Some(entry) = state.entries.get_mut(&(account_id, model_key.to_string())) else {
        return false;
    };
    if entry.generation != generation || !entry.in_flight {
        return false;
    }
    entry.pending_failure_cooldown |= cooldown;
    true
}

pub(crate) fn defer_priority_handoff_failure_for_attempt_or_invoke(
    attempt_id: Option<i64>,
    invoke_id: Option<&str>,
    cooldown: bool,
) -> bool {
    let context = attempt_id
        .and_then(priority_handoff_attempt_context)
        .or_else(|| {
            invoke_id
                .filter(|value| !value.is_empty())
                .and_then(priority_handoff_invoke_context)
        });
    let Some(context) = context else {
        return false;
    };
    defer_priority_handoff_failure_for_key(
        context.account_id,
        context.model_key.as_str(),
        context.generation,
        cooldown,
    )
}

pub(crate) fn release_priority_handoff_for_key(account_id: i64, model_key: &str, generation: u64) {
    let Ok(mut state) = state().lock() else {
        return;
    };
    let Some(entry) = state.entries.get_mut(&(account_id, model_key.to_string())) else {
        return;
    };
    if entry.in_flight_generation == Some(generation) {
        entry.in_flight = false;
        entry.in_flight_generation = None;
        if entry.pending_failure_cooldown {
            let _ = apply_failure_cooldown(entry);
            entry.pending_failure_cooldown = false;
        }
    }
}

pub(crate) async fn complete_priority_handoff_from_attempt(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    success: bool,
    cooldown: bool,
) {
    complete_priority_handoff_from_attempt_inner(
        pool, attempt_id, success, cooldown, false, None, false,
    )
    .await;
}

async fn prepare_successful_priority_handoff_attempt(
    pool: &Pool<Sqlite>,
    attempt_id: i64,
    cooldown: bool,
    defer_failure: bool,
    persist_admitted: bool,
) -> bool {
    let finalized = match sqlx::query_as::<_, (Option<String>, Option<i64>, Option<String>)>(
        "SELECT status, downstream_http_status, failure_kind FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await
    {
        Ok(value) => value,
        Err(error) => {
            warn!(
                attempt_id,
                error = %error,
                "failed to load priority handoff attempt terminal status; releasing local permit"
            );
            if let Some(context) = take_priority_handoff_attempt(attempt_id) {
                release_priority_handoff_for_key(
                    context.account_id,
                    &context.model_key,
                    context.generation,
                );
            }
            return false;
        }
    };
    let Some((attempt_status, downstream_http_status, failure_kind)) = finalized else {
        if let Some(context) = take_priority_handoff_attempt(attempt_id) {
            release_priority_handoff_for_key(
                context.account_id,
                &context.model_key,
                context.generation,
            );
        }
        return false;
    };
    if attempt_status.as_deref() != Some(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS) {
        if attempt_status.as_deref() != Some(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            && let Some(context) = take_priority_handoff_attempt(attempt_id)
        {
            if defer_failure {
                let reason_code = complete_failure_for_key(
                    context.account_id,
                    &context.model_key,
                    context.generation,
                    cooldown,
                );
                if let Some(reason_code) = reason_code
                    && let Err(error) = persist_priority_handoff_event_for_completion(
                        pool,
                        context.account_id,
                        Some(attempt_id),
                        context.model_key.as_str(),
                        reason_code,
                        persist_admitted,
                    )
                    .await
                {
                    warn!(
                        account_id = context.account_id,
                        attempt_id,
                        error = %error,
                        reason_code,
                        "failed to persist priority handoff event"
                    );
                }
            } else {
                release_priority_handoff_for_key(
                    context.account_id,
                    &context.model_key,
                    context.generation,
                );
            }
        }
        return false;
    }
    if downstream_http_status.is_some() || failure_kind.is_some() {
        if let Some(context) = take_priority_handoff_attempt(attempt_id) {
            release_priority_handoff_for_key(
                context.account_id,
                &context.model_key,
                context.generation,
            );
        }
        return false;
    }
    true
}
