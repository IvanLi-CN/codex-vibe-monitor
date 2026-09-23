use super::*;

use std::{
    ffi::CString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(crate) const RAW_CAPTURE_CLOSE_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub(crate) const RAW_CAPTURE_RESUME_BYTES: u64 = 12 * 1024 * 1024 * 1024;
pub(crate) const RAW_CAPTURE_CLOSE_AVAILABLE_BYTES: u64 = 20 * 1024 * 1024 * 1024;
pub(crate) const RAW_CAPTURE_RESUME_AVAILABLE_BYTES: u64 = 30 * 1024 * 1024 * 1024;
const RAW_CAPTURE_RESERVATION_OVERHEAD_BYTES: u64 = 64 * 1024;
const RAW_CAPTURE_RESERVATION_EXPANSION_FACTOR: u64 = 2;

const CIRCUIT_STATE_UNKNOWN: &str = "unknown";
const CIRCUIT_STATE_CAPTURING: &str = "capturing";
const CIRCUIT_STATE_SUPPRESSED: &str = "storage_suppressed";
const CIRCUIT_REASON_INVENTORY_UNREADY: &str = "inventory_unready";
const CIRCUIT_REASON_RAW_STORE_LIMIT: &str = "raw_store_limit";
const CIRCUIT_REASON_FILESYSTEM_LOW: &str = "filesystem_low";
const CIRCUIT_REASON_BOTH: &str = "both";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawCaptureCircuitSnapshot {
    pub(crate) state: String,
    pub(crate) reason: Option<String>,
    pub(crate) inventory_state: String,
    pub(crate) raw_bytes: Option<u64>,
    pub(crate) spool_bytes: Option<u64>,
    pub(crate) physical_raw_bytes: Option<u64>,
    pub(crate) recovery_pending: bool,
    pub(crate) available_bytes: Option<u64>,
    pub(crate) reserved_bytes: u64,
    pub(crate) expired_backlog_count: Option<u64>,
    pub(crate) backlog_non_growing: Option<bool>,
    pub(crate) updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawCaptureAdmissionError {
    pub(crate) reason: &'static str,
}

#[derive(Debug)]
struct RawCaptureCircuitState {
    state: &'static str,
    reason: Option<&'static str>,
    inventory_state: String,
    raw_bytes: Option<u64>,
    spool_bytes: Option<u64>,
    available_bytes: Option<u64>,
    reserved_bytes: u64,
    expired_backlog_count: Option<u64>,
    backlog_non_growing: Option<bool>,
    updated_at: Option<String>,
    admission_initialized: bool,
    inventory_generation: u64,
    accounting_generation: u64,
    resume_hysteresis: bool,
    resume_reason: Option<&'static str>,
    recovery_pending: bool,
}

impl Default for RawCaptureCircuitState {
    fn default() -> Self {
        let test_mode = cfg!(test);
        Self {
            state: if test_mode {
                CIRCUIT_STATE_CAPTURING
            } else {
                CIRCUIT_STATE_UNKNOWN
            },
            reason: if test_mode {
                None
            } else {
                Some(CIRCUIT_REASON_INVENTORY_UNREADY)
            },
            inventory_state: if test_mode {
                "ready".to_string()
            } else {
                "preparing".to_string()
            },
            raw_bytes: test_mode.then_some(0),
            spool_bytes: test_mode.then_some(0),
            available_bytes: test_mode.then_some(u64::MAX),
            reserved_bytes: 0,
            expired_backlog_count: None,
            backlog_non_growing: test_mode.then_some(true),
            updated_at: None,
            admission_initialized: test_mode,
            inventory_generation: 0,
            accounting_generation: 0,
            resume_hysteresis: false,
            resume_reason: None,
            recovery_pending: false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct RawCaptureCircuitBreaker {
    raw_root: PathBuf,
    state: Mutex<RawCaptureCircuitState>,
}

impl RawCaptureCircuitBreaker {
    pub(crate) fn new(raw_root: PathBuf) -> Self {
        Self {
            raw_root,
            state: Mutex::new(RawCaptureCircuitState::default()),
        }
    }

    pub(crate) fn snapshot(&self) -> RawCaptureCircuitSnapshot {
        let state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        RawCaptureCircuitSnapshot {
            state: state.state.to_string(),
            reason: state.reason.map(str::to_string),
            inventory_state: state.inventory_state.clone(),
            raw_bytes: state.raw_bytes,
            spool_bytes: state.spool_bytes,
            physical_raw_bytes: state
                .raw_bytes
                .map(|bytes| bytes.saturating_add(state.spool_bytes.unwrap_or_default())),
            recovery_pending: state.recovery_pending,
            available_bytes: state.available_bytes,
            reserved_bytes: state.reserved_bytes,
            expired_backlog_count: state.expired_backlog_count,
            backlog_non_growing: state.backlog_non_growing,
            updated_at: state.updated_at.clone(),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Hydration mirrors the additive durable circuit columns one-for-one."
    )]
    pub(crate) fn hydrate(
        &self,
        inventory_state: &str,
        circuit_state: Option<&str>,
        circuit_reason: Option<&str>,
        raw_bytes: u64,
        spool_bytes: Option<u64>,
        available_bytes: Option<u64>,
        expired_backlog_count: Option<u64>,
        backlog_non_growing: Option<bool>,
        updated_at: Option<String>,
    ) {
        self.hydrate_with_recovery_pending(
            inventory_state,
            circuit_state,
            circuit_reason,
            raw_bytes,
            spool_bytes,
            available_bytes,
            expired_backlog_count,
            backlog_non_growing,
            false,
            updated_at,
        );
    }

    pub(crate) fn hydrate_with_recovery_pending(
        &self,
        inventory_state: &str,
        circuit_state: Option<&str>,
        circuit_reason: Option<&str>,
        _raw_bytes: u64,
        _spool_bytes: Option<u64>,
        _available_bytes: Option<u64>,
        _expired_backlog_count: Option<u64>,
        _backlog_non_growing: Option<bool>,
        recovery_pending: bool,
        updated_at: Option<String>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        state.inventory_generation = state.inventory_generation.saturating_add(1);
        state.inventory_state = if inventory_state == "ready" {
            "preparing".to_string()
        } else {
            inventory_state.to_string()
        };
        state.updated_at = updated_at;
        state.raw_bytes = None;
        state.spool_bytes = None;
        state.available_bytes = None;
        state.expired_backlog_count = _expired_backlog_count;
        state.backlog_non_growing = _backlog_non_growing;
        state.resume_hysteresis = circuit_state == Some(CIRCUIT_STATE_SUPPRESSED);
        state.resume_reason = normalize_reason(circuit_reason);
        state.recovery_pending = recovery_pending || state.resume_hysteresis;
        state.state = CIRCUIT_STATE_UNKNOWN;
        state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
        state.admission_initialized = false;
    }

    pub(crate) fn inventory_checkpoint(&self) -> (u64, u64, Option<u64>) {
        let state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        (
            state.inventory_generation,
            state.accounting_generation,
            state.raw_bytes,
        )
    }

    pub(crate) fn update_inventory(
        &self,
        inventory_state: &str,
        raw_bytes: u64,
        available_bytes: Option<u64>,
        expired_backlog_count: Option<u64>,
    ) {
        let (generation, accounting_generation, _) = self.inventory_checkpoint();
        let _ = self.update_inventory_if_current(
            generation,
            accounting_generation,
            inventory_state,
            raw_bytes,
            available_bytes,
            expired_backlog_count,
            None,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Inventory reconciliation carries the bounded scan and generation guards together."
    )]
    pub(crate) fn update_inventory_if_current(
        &self,
        generation: u64,
        accounting_generation: u64,
        inventory_state: &str,
        raw_bytes: u64,
        available_bytes: Option<u64>,
        expired_backlog_count: Option<u64>,
        spool_bytes: Option<u64>,
    ) -> bool {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        if state.inventory_generation != generation {
            return false;
        }
        state.inventory_state = inventory_state.to_string();
        state.updated_at = Some(Utc::now().to_rfc3339());
        if inventory_state != "ready" {
            state.raw_bytes = None;
            state.spool_bytes = spool_bytes;
            state.available_bytes = if cfg!(test) {
                available_bytes.or(Some(u64::MAX))
            } else {
                filesystem_available_bytes(&self.raw_root)
            };
            state.backlog_non_growing = None;
            state.admission_initialized = false;
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            return true;
        }
        let awaiting_backlog_comparison = state.resume_hysteresis;
        state.backlog_non_growing = match (state.expired_backlog_count, expired_backlog_count) {
            (Some(previous), Some(current)) => Some(current <= previous),
            (None, Some(_)) if !awaiting_backlog_comparison => Some(true),
            _ => None,
        };
        if expired_backlog_count.is_some() {
            state.expired_backlog_count = expired_backlog_count;
        }
        state.admission_initialized = true;
        state.raw_bytes = Some(if state.accounting_generation == accounting_generation {
            raw_bytes
        } else {
            raw_bytes.max(state.raw_bytes.unwrap_or_default())
        });
        state.spool_bytes = spool_bytes;
        state.available_bytes = if cfg!(test) {
            available_bytes.or(state.available_bytes).or(Some(u64::MAX))
        } else {
            filesystem_available_bytes(&self.raw_root)
        };
        let reopening = state.resume_hysteresis;
        if reopening {
            state.state = CIRCUIT_STATE_SUPPRESSED;
            state.reason = state.resume_reason;
            if state.backlog_non_growing == Some(true) {
                state.resume_hysteresis = false;
                state.resume_reason = None;
                state.recovery_pending = false;
            }
        }
        evaluate_locked(&mut state, 0, reopening);
        true
    }

    pub(crate) fn mark_inventory_preparing(&self) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        let preserve_suppression = state.state == CIRCUIT_STATE_SUPPRESSED
            || state.resume_hysteresis
            || state.reason.is_some_and(|reason| {
                matches!(
                    reason,
                    CIRCUIT_REASON_RAW_STORE_LIMIT
                        | CIRCUIT_REASON_FILESYSTEM_LOW
                        | CIRCUIT_REASON_BOTH
                )
            });
        if preserve_suppression {
            state.resume_hysteresis = true;
            state.recovery_pending = true;
            state.resume_reason = state
                .resume_reason
                .or(state.reason)
                .or(Some(CIRCUIT_REASON_RAW_STORE_LIMIT));
        }
        state.inventory_generation = state.inventory_generation.saturating_add(1);
        state.inventory_state = "preparing".to_string();
        state.admission_initialized = false;
        state.raw_bytes = None;
        state.spool_bytes = None;
        state.available_bytes = None;
        state.backlog_non_growing = None;
        state.state = CIRCUIT_STATE_UNKNOWN;
        state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
        state.updated_at = Some(Utc::now().to_rfc3339());
    }

    pub(crate) fn admit(
        self: &Arc<Self>,
        requested_bytes: u64,
    ) -> Result<RawCaptureReservation, RawCaptureAdmissionError> {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        if !cfg!(test) {
            state.available_bytes = filesystem_available_bytes(&self.raw_root);
        }
        if state.inventory_state != "ready" {
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_INVENTORY_UNREADY,
            });
        }
        if !state.admission_initialized {
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_INVENTORY_UNREADY,
            });
        }
        if state.state == CIRCUIT_STATE_SUPPRESSED {
            evaluate_locked(&mut state, 0, true);
            if state.state == CIRCUIT_STATE_SUPPRESSED {
                return Err(RawCaptureAdmissionError {
                    reason: state.reason.unwrap_or(CIRCUIT_REASON_INVENTORY_UNREADY),
                });
            }
        }
        let reservation_bytes = capture_reservation_bytes(requested_bytes);
        evaluate_locked(&mut state, reservation_bytes, false);
        if state.state != CIRCUIT_STATE_CAPTURING {
            return Err(RawCaptureAdmissionError {
                reason: state.reason.unwrap_or(CIRCUIT_REASON_INVENTORY_UNREADY),
            });
        }
        state.reserved_bytes = state.reserved_bytes.saturating_add(reservation_bytes);
        Ok(RawCaptureReservation {
            circuit: self.clone(),
            reserved_bytes: reservation_bytes,
            finished: false,
        })
    }

    pub(crate) fn admit_recovery(
        self: &Arc<Self>,
        requested_bytes: u64,
    ) -> Result<RawCaptureReservation, RawCaptureAdmissionError> {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        if !cfg!(test) {
            state.available_bytes = filesystem_available_bytes(&self.raw_root);
        }
        let spool_only_recovery = state.inventory_state != "ready"
            && state.spool_bytes.is_some()
            && state.available_bytes.is_some();
        if !spool_only_recovery
            && (state.inventory_state != "ready"
                || !state.admission_initialized
                || state.raw_bytes.is_none()
                || state.available_bytes.is_none()
                || state.backlog_non_growing != Some(true))
        {
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_INVENTORY_UNREADY,
            });
        }
        let reservation_bytes = capture_reservation_bytes(requested_bytes);
        let replacement_bytes =
            reservation_bytes.saturating_sub(state.spool_bytes.unwrap_or_default());
        let projected_raw = state
            .raw_bytes
            .unwrap_or_default()
            .saturating_add(state.spool_bytes.unwrap_or_default())
            .saturating_add(state.reserved_bytes)
            .saturating_add(replacement_bytes);
        let replacement_only = requested_bytes <= state.spool_bytes.unwrap_or_default()
            && (spool_only_recovery
                || matches!(
                    state.reason,
                    Some(CIRCUIT_REASON_RAW_STORE_LIMIT | CIRCUIT_REASON_BOTH)
                ));
        if projected_raw >= RAW_CAPTURE_CLOSE_BYTES && !replacement_only {
            state.reason = Some(CIRCUIT_REASON_RAW_STORE_LIMIT);
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_RAW_STORE_LIMIT,
            });
        }
        if state.available_bytes.is_some_and(|bytes| {
            bytes
                .saturating_sub(state.reserved_bytes)
                .saturating_sub(reservation_bytes)
                <= RAW_CAPTURE_CLOSE_AVAILABLE_BYTES
        }) {
            state.reason = Some(CIRCUIT_REASON_FILESYSTEM_LOW);
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_FILESYSTEM_LOW,
            });
        }
        state.reserved_bytes = state.reserved_bytes.saturating_add(reservation_bytes);
        Ok(RawCaptureReservation {
            circuit: self.clone(),
            reserved_bytes: reservation_bytes,
            finished: false,
        })
    }

    pub(crate) fn record_deleted_bytes(&self, bytes: u64) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        if let Some(raw_bytes) = state.raw_bytes.as_mut() {
            *raw_bytes = raw_bytes.saturating_sub(bytes);
        }
        if bytes > 0 {
            state.accounting_generation = state.accounting_generation.saturating_add(1);
        }
        if !cfg!(test) {
            state.available_bytes = filesystem_available_bytes(&self.raw_root);
        }
        state.updated_at = Some(Utc::now().to_rfc3339());
        evaluate_locked(&mut state, 0, false);
    }

    fn finish_reservation(&self, reserved_bytes: u64, actual_bytes: u64) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        state.reserved_bytes = state.reserved_bytes.saturating_sub(reserved_bytes);
        if let Some(raw_bytes) = state.raw_bytes.as_mut() {
            *raw_bytes = raw_bytes.saturating_add(actual_bytes);
        }
        if actual_bytes > 0 {
            state.accounting_generation = state.accounting_generation.saturating_add(1);
        }
        if !cfg!(test) {
            state.available_bytes = filesystem_available_bytes(&self.raw_root);
        }
        state.updated_at = Some(Utc::now().to_rfc3339());
        evaluate_locked(&mut state, 0, false);
    }

    fn release_reservation(&self, reserved_bytes: u64) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        state.reserved_bytes = state.reserved_bytes.saturating_sub(reserved_bytes);
        if !cfg!(test) {
            state.available_bytes = filesystem_available_bytes(&self.raw_root);
        }
    }

    fn reserve_additional(&self, requested_bytes: u64) -> Result<(), RawCaptureAdmissionError> {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        if !cfg!(test) {
            state.available_bytes = filesystem_available_bytes(&self.raw_root);
        }
        if state.inventory_state != "ready" {
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_INVENTORY_UNREADY,
            });
        }
        if !state.admission_initialized {
            return Err(RawCaptureAdmissionError {
                reason: CIRCUIT_REASON_INVENTORY_UNREADY,
            });
        }
        let reservation_bytes = capture_extension_bytes(requested_bytes);
        evaluate_locked(&mut state, reservation_bytes, false);
        if state.state != CIRCUIT_STATE_CAPTURING {
            return Err(RawCaptureAdmissionError {
                reason: state.reason.unwrap_or(CIRCUIT_REASON_INVENTORY_UNREADY),
            });
        }
        state.reserved_bytes = state.reserved_bytes.saturating_add(reservation_bytes);
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct RawCaptureReservation {
    circuit: Arc<RawCaptureCircuitBreaker>,
    reserved_bytes: u64,
    finished: bool,
}

impl RawCaptureReservation {
    pub(crate) fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    pub(crate) fn extend(&mut self, additional_bytes: u64) -> Result<(), RawCaptureAdmissionError> {
        if additional_bytes == 0 {
            return Ok(());
        }
        self.circuit.reserve_additional(additional_bytes)?;
        self.reserved_bytes = self
            .reserved_bytes
            .saturating_add(capture_extension_bytes(additional_bytes));
        Ok(())
    }

    pub(crate) fn finish(mut self, actual_bytes: u64) {
        self.finished = true;
        self.circuit
            .finish_reservation(self.reserved_bytes, actual_bytes);
    }
}

impl Drop for RawCaptureReservation {
    fn drop(&mut self) {
        if !self.finished {
            self.circuit.release_reservation(self.reserved_bytes);
        }
    }
}

fn capture_reservation_bytes(requested_bytes: u64) -> u64 {
    capture_extension_bytes(requested_bytes).saturating_add(RAW_CAPTURE_RESERVATION_OVERHEAD_BYTES)
}

fn capture_extension_bytes(requested_bytes: u64) -> u64 {
    requested_bytes.saturating_mul(RAW_CAPTURE_RESERVATION_EXPANSION_FACTOR)
}

fn evaluate_locked(state: &mut RawCaptureCircuitState, requested_bytes: u64, reopening: bool) {
    if state.inventory_state != "ready" {
        state.state = CIRCUIT_STATE_UNKNOWN;
        state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
        return;
    }
    if state.available_bytes.is_none() {
        state.state = CIRCUIT_STATE_UNKNOWN;
        state.reason = Some(CIRCUIT_REASON_FILESYSTEM_LOW);
        return;
    }
    if state.raw_bytes.is_none() {
        state.state = CIRCUIT_STATE_UNKNOWN;
        state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
        return;
    }
    if state.backlog_non_growing != Some(true) {
        state.state = CIRCUIT_STATE_UNKNOWN;
        state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
        return;
    }
    let projected_raw = state
        .raw_bytes
        .unwrap_or_default()
        .saturating_add(state.spool_bytes.unwrap_or_default())
        .saturating_add(state.reserved_bytes)
        .saturating_add(requested_bytes);
    let raw_limited = projected_raw >= RAW_CAPTURE_CLOSE_BYTES;
    let filesystem_limited = state.available_bytes.is_some_and(|bytes| {
        bytes
            .saturating_sub(state.reserved_bytes)
            .saturating_sub(requested_bytes)
            <= RAW_CAPTURE_CLOSE_AVAILABLE_BYTES
    });
    if state.state == CIRCUIT_STATE_SUPPRESSED || reopening {
        let backlog_clear = state.backlog_non_growing == Some(true);
        let resumed = state
            .raw_bytes
            .unwrap_or_default()
            .saturating_add(state.spool_bytes.unwrap_or_default())
            .saturating_add(state.reserved_bytes)
            < RAW_CAPTURE_RESUME_BYTES
            && state.available_bytes.is_some_and(|bytes| {
                bytes.saturating_sub(state.reserved_bytes) >= RAW_CAPTURE_RESUME_AVAILABLE_BYTES
            })
            && backlog_clear;
        if !resumed {
            state.state = CIRCUIT_STATE_SUPPRESSED;
            state.reason = Some(if raw_limited && filesystem_limited {
                CIRCUIT_REASON_BOTH
            } else if raw_limited {
                CIRCUIT_REASON_RAW_STORE_LIMIT
            } else if filesystem_limited {
                CIRCUIT_REASON_FILESYSTEM_LOW
            } else {
                state.reason.unwrap_or(CIRCUIT_REASON_INVENTORY_UNREADY)
            });
            return;
        }
    }
    if raw_limited || filesystem_limited {
        state.state = CIRCUIT_STATE_SUPPRESSED;
        state.reason = Some(if raw_limited && filesystem_limited {
            CIRCUIT_REASON_BOTH
        } else if raw_limited {
            CIRCUIT_REASON_RAW_STORE_LIMIT
        } else {
            CIRCUIT_REASON_FILESYSTEM_LOW
        });
    } else {
        state.state = CIRCUIT_STATE_CAPTURING;
        state.reason = None;
    }
}

fn normalize_reason(reason: Option<&str>) -> Option<&'static str> {
    match reason {
        Some(CIRCUIT_REASON_RAW_STORE_LIMIT) => Some(CIRCUIT_REASON_RAW_STORE_LIMIT),
        Some(CIRCUIT_REASON_FILESYSTEM_LOW) => Some(CIRCUIT_REASON_FILESYSTEM_LOW),
        Some(CIRCUIT_REASON_BOTH) => Some(CIRCUIT_REASON_BOTH),
        Some(CIRCUIT_REASON_INVENTORY_UNREADY) => Some(CIRCUIT_REASON_INVENTORY_UNREADY),
        _ => None,
    }
}

pub(crate) fn filesystem_available_bytes(root: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut candidate = root;
        while !candidate.exists() {
            candidate = candidate.parent()?;
        }
        let path = CString::new(candidate.as_os_str().as_bytes()).ok()?;
        let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
        if result != 0 {
            return None;
        }
        let stats = unsafe { stats.assume_init() };
        let blocks = u128::from(stats.f_bavail);
        let fragment_size = u128::from(stats.f_frsize);
        u64::try_from(blocks.saturating_mul(fragment_size)).ok()
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready(
        raw_bytes: u64,
        available_bytes: u64,
        backlog_non_growing: bool,
    ) -> Arc<RawCaptureCircuitBreaker> {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.hydrate(
            "ready",
            Some(CIRCUIT_STATE_CAPTURING),
            None,
            raw_bytes,
            None,
            Some(available_bytes),
            Some(0),
            Some(backlog_non_growing),
            None,
        );
        circuit.update_inventory("ready", raw_bytes, Some(available_bytes), Some(0));
        circuit
    }

    #[test]
    fn closes_at_raw_watermark_and_reopens_only_after_hysteresis() {
        let circuit = ready(
            RAW_CAPTURE_CLOSE_BYTES - 1,
            RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
            true,
        );
        let reservation = circuit
            .admit(1)
            .expect_err("close watermark must suppress capture");
        assert_eq!(reservation.reason, CIRCUIT_REASON_RAW_STORE_LIMIT);

        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES - 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
        );
        assert!(circuit.admit(0).is_ok());
    }

    #[test]
    fn unknown_inventory_fails_closed() {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.mark_inventory_preparing();
        let error = circuit
            .admit(1)
            .expect_err("unknown inventory must suppress capture");
        assert_eq!(error.reason, CIRCUIT_REASON_INVENTORY_UNREADY);
    }

    #[test]
    fn unknown_backlog_history_fails_closed() {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.hydrate(
            "ready",
            Some(CIRCUIT_STATE_CAPTURING),
            None,
            0,
            None,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(3),
            None,
            None,
        );
        let snapshot = circuit.snapshot();
        assert_eq!(snapshot.state, CIRCUIT_STATE_UNKNOWN);
        assert_eq!(
            snapshot.reason,
            Some(CIRCUIT_REASON_INVENTORY_UNREADY.to_string())
        );
        assert_eq!(
            circuit
                .admit(1)
                .expect_err("unknown backlog must suppress capture")
                .reason,
            CIRCUIT_REASON_INVENTORY_UNREADY
        );
    }

    #[test]
    fn unrecognized_persisted_state_fails_closed() {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.hydrate(
            "ready",
            Some("garbled"),
            None,
            0,
            None,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
            Some(true),
            None,
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_UNKNOWN);
        assert_eq!(
            circuit
                .admit(1)
                .expect_err("unrecognized state must suppress capture")
                .reason,
            CIRCUIT_REASON_INVENTORY_UNREADY
        );
    }

    #[test]
    fn reservations_count_against_the_close_watermark() {
        let circuit = ready(
            RAW_CAPTURE_CLOSE_BYTES - capture_reservation_bytes(9) - 1,
            RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
            true,
        );
        let _reservation = circuit.admit(9).expect("first reservation should fit");
        let error = circuit
            .admit(2)
            .expect_err("second reservation crosses close watermark");
        assert_eq!(error.reason, CIRCUIT_REASON_RAW_STORE_LIMIT);
    }

    #[test]
    fn inventory_refresh_keeps_concurrent_completed_bytes() {
        let circuit = ready(100, RAW_CAPTURE_RESUME_AVAILABLE_BYTES, true);
        let (generation, accounting_generation, _) = circuit.inventory_checkpoint();
        let reservation = circuit.admit(20).expect("reservation should fit");
        reservation.finish(20);

        assert!(circuit.update_inventory_if_current(
            generation,
            accounting_generation,
            "ready",
            100,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
            None,
        ));
        assert_eq!(circuit.snapshot().raw_bytes, Some(120));
    }

    #[test]
    fn reset_invalidates_an_in_flight_inventory_refresh() {
        let circuit = ready(100, RAW_CAPTURE_RESUME_AVAILABLE_BYTES, true);
        let (generation, accounting_generation, _) = circuit.inventory_checkpoint();
        circuit.mark_inventory_preparing();
        assert!(!circuit.update_inventory_if_current(
            generation,
            accounting_generation,
            "ready",
            100,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
            None,
        ));
        assert_eq!(circuit.snapshot().inventory_state, "preparing");
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_UNKNOWN);
    }

    #[test]
    fn hydrated_inventory_stays_fail_closed_until_a_fresh_scan() {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.hydrate(
            "ready",
            Some(CIRCUIT_STATE_CAPTURING),
            None,
            0,
            None,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
            Some(true),
            None,
        );
        assert_eq!(
            circuit
                .admit(1)
                .expect_err("hydration must remain fail-closed")
                .reason,
            CIRCUIT_REASON_INVENTORY_UNREADY
        );
        circuit.update_inventory(
            "ready",
            0,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
        );
        assert!(circuit.admit(1).is_ok());
    }

    #[test]
    fn suppressed_reason_is_preserved_between_hysteresis_watermarks() {
        let circuit = ready(
            RAW_CAPTURE_CLOSE_BYTES,
            RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
            true,
        );
        let error = circuit
            .admit(0)
            .expect_err("raw watermark should suppress capture");
        assert_eq!(error.reason, CIRCUIT_REASON_RAW_STORE_LIMIT);

        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
        );
        let snapshot = circuit.snapshot();
        assert_eq!(snapshot.state, CIRCUIT_STATE_SUPPRESSED);
        assert_eq!(
            snapshot.reason.as_deref(),
            Some(CIRCUIT_REASON_RAW_STORE_LIMIT)
        );
    }

    #[test]
    fn persisted_suppression_keeps_hysteresis_after_restart() {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.hydrate(
            "ready",
            Some(CIRCUIT_STATE_SUPPRESSED),
            Some(CIRCUIT_REASON_RAW_STORE_LIMIT),
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(0),
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES - 1),
            Some(0),
            Some(true),
            None,
        );
        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES - 1),
            Some(0),
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_SUPPRESSED);

        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES - 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_CAPTURING);
    }

    #[test]
    fn inventory_reset_keeps_suppression_hysteresis() {
        let circuit = ready(
            RAW_CAPTURE_CLOSE_BYTES,
            RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
            true,
        );
        assert_eq!(
            circuit
                .admit(0)
                .expect_err("close watermark should suppress capture")
                .reason,
            CIRCUIT_REASON_RAW_STORE_LIMIT
        );
        circuit.mark_inventory_preparing();
        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES - 1),
            Some(0),
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_SUPPRESSED);
        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES - 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(0),
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_CAPTURING);
    }

    #[test]
    fn suppressed_restart_waits_for_known_backlog_comparison() {
        let circuit = Arc::new(RawCaptureCircuitBreaker::new(PathBuf::from("/")));
        circuit.hydrate(
            "ready",
            Some(CIRCUIT_STATE_SUPPRESSED),
            Some(CIRCUIT_REASON_RAW_STORE_LIMIT),
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(0),
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES + 1),
            Some(5),
            Some(true),
            None,
        );
        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES + 1),
            None,
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_UNKNOWN);
        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES + 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES + 1),
            Some(5),
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_SUPPRESSED);
        circuit.update_inventory(
            "ready",
            RAW_CAPTURE_RESUME_BYTES - 1,
            Some(RAW_CAPTURE_RESUME_AVAILABLE_BYTES),
            Some(5),
        );
        assert_eq!(circuit.snapshot().state, CIRCUIT_STATE_CAPTURING);
    }
}
