use super::*;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub(crate) const PRIORITY_HANDOFF_ROUTING_SOURCE: &str = "priorityHandoff";
pub(crate) const PRIORITY_HANDOFF_SUCCEEDED_REASON: &str = "priorityHandoffSucceeded";
pub(crate) const PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON: &str = "priorityHandoffFailureCooldown";
pub(crate) const PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON: &str =
    "priorityHandoffRecoveryProgress";
const PRIORITY_HANDOFF_VERIFICATION_SUCCESSES: u8 = 3;
const PRIORITY_HANDOFF_FIRST_COOLDOWN_SECS: u64 = 5;
const PRIORITY_HANDOFF_MAX_COOLDOWN_SECS: u64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PriorityHandoffPhase {
    Verifying,
    Open,
    CoolingDown,
}

#[derive(Debug)]
struct PriorityHandoffEntry {
    generation: u64,
    phase: PriorityHandoffPhase,
    verification_successes: u8,
    failure_streak: u32,
    cooldown_until: Option<Instant>,
    in_flight: bool,
}

#[derive(Debug, Default)]
struct PriorityHandoffState {
    enabled: bool,
    generation: u64,
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
            entries: HashMap::new(),
        }
    }
}

fn state() -> &'static Mutex<PriorityHandoffState> {
    static STATE: OnceLock<Mutex<PriorityHandoffState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(PriorityHandoffState::new()))
}

fn attempt_contexts() -> &'static Mutex<HashMap<i64, PriorityHandoffAttemptContext>> {
    static ATTEMPT_CONTEXTS: OnceLock<Mutex<HashMap<i64, PriorityHandoffAttemptContext>>> =
        OnceLock::new();
    ATTEMPT_CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn normalize_model_key(model: Option<&str>) -> Option<String> {
    model
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
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
    account_id: i64,
    requested_model: Option<&str>,
    audit_json: Option<&str>,
) {
    let (Some(attempt_id), Some(model_key), Some(generation)) = (
        attempt_id,
        normalize_model_key(requested_model),
        priority_handoff_generation_from_audit_json(audit_json),
    ) else {
        return;
    };
    if let Ok(mut contexts) = attempt_contexts().lock() {
        contexts.insert(
            attempt_id,
            PriorityHandoffAttemptContext {
                account_id,
                model_key,
                generation,
            },
        );
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

pub(crate) fn forget_priority_handoff_attempt(attempt_id: Option<i64>) {
    if let Some(attempt_id) = attempt_id {
        let _ = take_priority_handoff_attempt(attempt_id);
    }
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
}

impl PriorityHandoffPermit {
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
            release_for_key(self.account_id, &self.model_key, self.generation);
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
    state.generation = state.generation.saturating_add(1);
    for entry in state.entries.values_mut() {
        entry.in_flight = false;
    }
    if enabled {
        let generation = state.generation;
        for entry in state.entries.values_mut() {
            entry.phase = PriorityHandoffPhase::Verifying;
            entry.verification_successes = 0;
            entry.failure_streak = 0;
            entry.cooldown_until = None;
            entry.generation = generation;
        }
    }
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
    let generation = state.generation;
    let entry = state
        .entries
        .entry((account_id, model_key.clone()))
        .or_insert_with(|| PriorityHandoffEntry {
            generation,
            phase: PriorityHandoffPhase::Verifying,
            verification_successes: 0,
            failure_streak: 0,
            cooldown_until: None,
            in_flight: false,
        });
    if entry.generation != generation {
        entry.generation = generation;
        entry.phase = PriorityHandoffPhase::Verifying;
        entry.verification_successes = 0;
        entry.failure_streak = 0;
        entry.cooldown_until = None;
        entry.in_flight = false;
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
    (
        PriorityHandoffAdmissionDecision::Admitted { generation },
        Some(Arc::new(PriorityHandoffPermit {
            account_id,
            model_key,
            generation,
            completed: AtomicBool::new(false),
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
            generation,
            phase: PriorityHandoffPhase::Verifying,
            verification_successes: 0,
            failure_streak: 0,
            cooldown_until: None,
            in_flight: false,
        });
    if entry.generation != generation {
        entry.generation = generation;
        entry.phase = PriorityHandoffPhase::Verifying;
        entry.verification_successes = 0;
        entry.failure_streak = 0;
        entry.cooldown_until = None;
        entry.in_flight = false;
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
        return None;
    }
    if !entry.in_flight {
        return None;
    }
    entry.in_flight = false;
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
    if entry.generation != generation || !entry.in_flight {
        return None;
    }
    entry.in_flight = false;
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
    if entry.generation != generation || !entry.in_flight {
        return None;
    }
    entry.in_flight = false;
    if !cooldown {
        entry.phase = PriorityHandoffPhase::Verifying;
        return None;
    }
    entry.failure_streak = entry.failure_streak.saturating_add(1);
    let shift = entry.failure_streak.saturating_sub(1).min(8);
    let cooldown_secs = PRIORITY_HANDOFF_FIRST_COOLDOWN_SECS
        .saturating_mul(1_u64 << shift)
        .min(PRIORITY_HANDOFF_MAX_COOLDOWN_SECS);
    entry.cooldown_until = Some(Instant::now() + Duration::from_secs(cooldown_secs));
    entry.phase = PriorityHandoffPhase::CoolingDown;
    entry.verification_successes = 0;
    Some(PRIORITY_HANDOFF_FAILURE_COOLDOWN_REASON)
}

fn release_for_key(account_id: i64, model_key: &str, generation: u64) {
    let Ok(mut state) = state().lock() else {
        return;
    };
    let Some(entry) = state.entries.get_mut(&(account_id, model_key.to_string())) else {
        return;
    };
    if entry.generation == generation {
        entry.in_flight = false;
    }
}

pub(crate) async fn complete_priority_handoff_from_attempt(
    pool: &Pool<Sqlite>,
    attempt_id: Option<i64>,
    success: bool,
    cooldown: bool,
) {
    let Some(attempt_id) = attempt_id else {
        return;
    };
    if success {
        // Success may be recorded before the streaming finalizer runs. Defer
        // in that case so a later downstream disconnect cannot become success
        // evidence; a finalized, non-cancelled attempt is safe to complete.
        let finalized = sqlx::query_as::<_, (
            Option<String>,
            Option<i64>,
            Option<String>,
        )>(
            "SELECT status, downstream_http_status, failure_kind FROM pool_upstream_request_attempts WHERE id = ?1",
        )
        .bind(attempt_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let Some((attempt_status, downstream_http_status, failure_kind)) = finalized else {
            if let Some(context) = take_priority_handoff_attempt(attempt_id) {
                release_for_key(context.account_id, &context.model_key, context.generation);
            }
            return;
        };
        if attempt_status.as_deref() != Some(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS) {
            if attempt_status.as_deref() != Some(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING) {
                if let Some(context) = take_priority_handoff_attempt(attempt_id) {
                    release_for_key(context.account_id, &context.model_key, context.generation);
                }
            }
            return;
        }
        if downstream_http_status.is_some() || failure_kind.is_some() {
            if let Some(context) = take_priority_handoff_attempt(attempt_id) {
                release_for_key(context.account_id, &context.model_key, context.generation);
            }
            return;
        }
    }
    if let Some(context) = take_priority_handoff_attempt(attempt_id) {
        let reason_code = complete_priority_handoff_for_request(
            context.account_id,
            Some(context.model_key.as_str()),
            Some(context.generation),
            success,
            cooldown,
        );
        if let Some(reason_code) = reason_code
            && let Err(error) = super::model_health::persist_priority_handoff_event(
                pool,
                context.account_id,
                Some(attempt_id),
                context.model_key.as_str(),
                reason_code,
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
        return;
    }
    let context = sqlx::query_as::<_, (
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    )>(
        "SELECT routing_source, request_model, downstream_http_status, status, failure_kind, routing_selection_audit_json, upstream_account_id FROM pool_upstream_request_attempts WHERE id = ?1",
    )
    .bind(attempt_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((
        Some(source),
        model,
        downstream_http_status,
        attempt_status,
        failure_kind,
        routing_selection_audit_json,
        account_id,
    )) = context
    else {
        return;
    };
    if source != PRIORITY_HANDOFF_ROUTING_SOURCE {
        return;
    }
    if attempt_status.as_deref() == Some(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        && downstream_http_status.is_some()
        && failure_kind.is_none()
    {
        // A finalized successful attempt with a downstream status is the
        // persisted shape of a pure client cancellation. It carries no
        // evidence for the handoff state machine.
        return;
    }
    let Some(model_key) = normalize_model_key(model.as_deref()) else {
        return;
    };
    let Some(account_id) = account_id else {
        return;
    };
    let Some(generation) =
        priority_handoff_generation_from_audit_json(routing_selection_audit_json.as_deref())
    else {
        return;
    };
    let reason_code = complete_priority_handoff_for_request(
        account_id,
        Some(model_key.as_str()),
        Some(generation),
        false,
        cooldown,
    );
    if let Some(reason_code) = reason_code
        && let Err(error) = super::model_health::persist_priority_handoff_event(
            pool,
            account_id,
            Some(attempt_id),
            model_key.as_str(),
            reason_code,
        )
        .await
    {
        warn!(
            account_id,
            attempt_id,
            error = %error,
            reason_code,
            "failed to persist priority handoff event"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("priority handoff test lock")
    }

    #[test]
    fn priority_handoff_admission_stateful() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (_, first) = admit_priority_handoff(9_001, Some("gpt-test"));
        assert!(first.is_some());
        let (busy, _second) = admit_priority_handoff(9_001, Some("gpt-test"));
        assert_eq!(busy, PriorityHandoffAdmissionDecision::PermitBusy);
        drop(first);
        let (_, third) = admit_priority_handoff(9_001, Some("gpt-test"));
        assert!(third.is_some());
    }

    #[test]
    fn priority_handoff_transport() {
        let _guard = test_guard();
        for _ in 0..3 {
            let (_, permit) = admit_priority_handoff(9_002, Some("gpt-test"));
            permit.expect("permit").complete_success();
        }
        let (_, next) = admit_priority_handoff(9_002, Some("gpt-test"));
        assert!(next.is_none());
        assert_eq!(
            priority_handoff_admission_snapshot(9_002, Some("gpt-test")).0,
            "open"
        );
    }

    #[test]
    fn priority_handoff_admission_is_isolated_by_model() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (model_a_decision, model_a_permit) = admit_priority_handoff(9_007, Some("model-a"));
        assert!(matches!(
            model_a_decision,
            PriorityHandoffAdmissionDecision::Admitted { .. }
        ));
        let (model_b_decision, model_b_permit) = admit_priority_handoff(9_007, Some("model-b"));
        assert!(matches!(
            model_b_decision,
            PriorityHandoffAdmissionDecision::Admitted { .. }
        ));
        let (model_a_busy, _) = admit_priority_handoff(9_007, Some("model-a"));
        assert_eq!(model_a_busy, PriorityHandoffAdmissionDecision::PermitBusy);
        drop(model_a_permit);
        drop(model_b_permit);
    }

    #[test]
    fn priority_handoff_client_cancellation_only_releases_permit() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (_, permit) = admit_priority_handoff(9_006, Some("gpt-test"));
        assert!(permit.is_some());
        assert!(priority_handoff_client_cancellation(
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            Some(StatusCode::OK),
            None,
        ));
        drop(permit);
        let (_, next) = admit_priority_handoff(9_006, Some("gpt-test"));
        assert!(next.is_some());
        assert_eq!(
            priority_handoff_admission_snapshot(9_006, Some("gpt-test")).0,
            "verifying"
        );
    }

    #[test]
    fn priority_handoff_failure_enters_cooldown_without_persistence() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (_, permit) = admit_priority_handoff(9_003, Some("gpt-test"));
        assert!(permit.is_some());
        complete_priority_handoff_for_request(9_003, Some("gpt-test"), None, false, true);
        assert_eq!(
            priority_handoff_admission_snapshot(9_003, Some("gpt-test")).0,
            "coolingDown"
        );
        drop(permit);
    }

    #[tokio::test]
    async fn priority_handoff_database_failure_does_not_block_local_transition() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (decision, permit) = admit_priority_handoff(9_004, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected admission");
        };
        assert!(permit.is_some());

        let audit_json = serde_json::json!({
            "selectedAccountId": 9_004,
            "selectedAccountName": "test",
            "eligibleCandidateCount": 1,
            "winnerReasonCode": "priorityHandoff",
            "comparedAccountId": null,
            "comparedAccountName": null,
            "handoffAdmission": {
                "decision": "admitted",
                "phase": "verifying",
                "verificationSuccessCount": 0,
                "generation": generation,
            },
            "excludedCandidates": [],
        })
        .to_string();
        remember_priority_handoff_attempt(
            Some(70_004),
            9_004,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("in-memory sqlite pool");
        complete_priority_handoff_from_attempt(&pool, Some(70_004), false, true).await;

        assert_eq!(
            priority_handoff_admission_snapshot(9_004, Some("gpt-test")).0,
            "coolingDown"
        );

        drop(permit);
    }

    #[tokio::test]
    async fn priority_handoff_finalized_success_advances_from_attempt_record() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (decision, permit) = admit_priority_handoff(9_008, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected admission");
        };
        assert!(permit.is_some());

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, status TEXT, downstream_http_status INTEGER, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create attempt table");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, status, downstream_http_status, failure_kind) VALUES (?1, ?2, NULL, NULL)",
        )
        .bind(70_008_i64)
        .bind(POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
        .execute(&pool)
        .await
        .expect("insert finalized successful attempt");
        let audit_json = serde_json::json!({
            "selectedAccountId": 9_008,
            "selectedAccountName": "test",
            "eligibleCandidateCount": 1,
            "winnerReasonCode": "priorityHandoff",
            "comparedAccountId": null,
            "comparedAccountName": null,
            "handoffAdmission": {
                "decision": "admitted",
                "phase": "verifying",
                "verificationSuccessCount": 0,
                "generation": generation,
            },
            "excludedCandidates": [],
        })
        .to_string();
        remember_priority_handoff_attempt(
            Some(70_008),
            9_008,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );

        complete_priority_handoff_from_attempt(&pool, Some(70_008), true, false).await;

        assert_eq!(
            priority_handoff_admission_snapshot(9_008, Some("gpt-test")),
            ("verifying".to_string(), 1)
        );
        drop(permit);
    }

    #[tokio::test]
    async fn priority_handoff_unknown_terminal_status_releases_attempt() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (decision, permit) = admit_priority_handoff(9_009, Some("gpt-test"));
        assert!(matches!(
            decision,
            PriorityHandoffAdmissionDecision::Admitted { .. }
        ));
        assert!(permit.is_some());

        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, status TEXT, downstream_http_status INTEGER, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create attempt table");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, status, downstream_http_status, failure_kind) VALUES (?1, NULL, NULL, NULL)",
        )
        .bind(70_009_i64)
        .execute(&pool)
        .await
        .expect("insert unknown terminal attempt");
        let generation = match decision {
            PriorityHandoffAdmissionDecision::Admitted { generation } => generation,
            _ => unreachable!(),
        };
        let audit_json = serde_json::json!({
            "selectedAccountId": 9_009,
            "selectedAccountName": "test",
            "eligibleCandidateCount": 1,
            "winnerReasonCode": "priorityHandoff",
            "comparedAccountId": null,
            "comparedAccountName": null,
            "handoffAdmission": {
                "decision": "admitted",
                "phase": "verifying",
                "verificationSuccessCount": 0,
                "generation": generation,
            },
            "excludedCandidates": [],
        })
        .to_string();
        remember_priority_handoff_attempt(
            Some(70_009),
            9_009,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );

        complete_priority_handoff_from_attempt(&pool, Some(70_009), true, false).await;

        let (_, next) = admit_priority_handoff(9_009, Some("gpt-test"));
        assert!(next.is_some());
        drop(permit);
        drop(next);
    }

    #[test]
    fn priority_handoff_generation_ignores_old_completion() {
        let _guard = test_guard();
        let (old_decision, old_permit) = admit_priority_handoff(9_005, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted {
            generation: old_generation,
        } = old_decision
        else {
            panic!("expected first generation admission");
        };

        let bumped_generation = {
            let mut state = state().lock().expect("priority handoff state");
            state.generation = state.generation.saturating_add(1);
            state.generation
        };

        let (new_decision, new_permit) = admit_priority_handoff(9_005, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted {
            generation: new_generation,
        } = new_decision
        else {
            panic!("expected new generation admission");
        };
        assert_ne!(old_generation, new_generation);
        assert_eq!(bumped_generation, new_generation);

        complete_priority_handoff_for_request(
            9_005,
            Some("gpt-test"),
            Some(old_generation),
            true,
            false,
        );
        assert_eq!(
            priority_handoff_admission_snapshot(9_005, Some("gpt-test")).1,
            0
        );
        drop(old_permit);
        drop(new_permit);
    }
}
