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
    available_bytes: Option<u64>,
    reserved_bytes: u64,
    expired_backlog_count: Option<u64>,
    backlog_non_growing: Option<bool>,
    updated_at: Option<String>,
    admission_initialized: bool,
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
            available_bytes: test_mode.then_some(u64::MAX),
            reserved_bytes: 0,
            expired_backlog_count: None,
            backlog_non_growing: test_mode.then_some(true),
            updated_at: None,
            admission_initialized: test_mode,
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
        available_bytes: Option<u64>,
        expired_backlog_count: Option<u64>,
        backlog_non_growing: Option<bool>,
        updated_at: Option<String>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        state.inventory_state = inventory_state.to_string();
        state.raw_bytes = Some(raw_bytes);
        state.available_bytes = if cfg!(test) {
            available_bytes.or(state.available_bytes)
        } else {
            filesystem_available_bytes(&self.raw_root)
        };
        state.expired_backlog_count = expired_backlog_count;
        state.backlog_non_growing = backlog_non_growing;
        state.updated_at = updated_at;
        if inventory_state != "ready" {
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            return;
        }
        let Some(persisted_state) = circuit_state
            .filter(|value| matches!(*value, CIRCUIT_STATE_SUPPRESSED | CIRCUIT_STATE_CAPTURING))
        else {
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            state.admission_initialized = false;
            return;
        };
        state.state = match persisted_state {
            CIRCUIT_STATE_SUPPRESSED => CIRCUIT_STATE_SUPPRESSED,
            CIRCUIT_STATE_CAPTURING => CIRCUIT_STATE_CAPTURING,
            _ => unreachable!("persisted state was validated above"),
        };
        state.admission_initialized = true;
        state.reason = normalize_reason(circuit_reason);
        evaluate_locked(&mut state, 0, false);
    }

    pub(crate) fn update_inventory(
        &self,
        inventory_state: &str,
        raw_bytes: u64,
        available_bytes: Option<u64>,
        expired_backlog_count: Option<u64>,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        if let (Some(previous), Some(current)) =
            (state.expired_backlog_count, expired_backlog_count)
        {
            state.backlog_non_growing = Some(current <= previous);
        } else {
            state.backlog_non_growing = None;
        }
        state.expired_backlog_count = expired_backlog_count;
        state.inventory_state = inventory_state.to_string();
        state.admission_initialized = true;
        state.raw_bytes = Some(raw_bytes);
        state.available_bytes = if cfg!(test) {
            available_bytes.or(state.available_bytes)
        } else {
            filesystem_available_bytes(&self.raw_root)
        };
        state.updated_at = Some(Utc::now().to_rfc3339());
        if inventory_state != "ready" {
            state.state = CIRCUIT_STATE_UNKNOWN;
            state.reason = Some(CIRCUIT_REASON_INVENTORY_UNREADY);
            return;
        }
        evaluate_locked(&mut state, 0, false);
    }

    pub(crate) fn mark_inventory_preparing(&self) {
        let mut state = self
            .state
            .lock()
            .expect("raw capture circuit mutex poisoned");
        state.inventory_state = "preparing".to_string();
        state.admission_initialized = false;
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
        evaluate_locked(&mut state, requested_bytes, false);
        if state.state != CIRCUIT_STATE_CAPTURING {
            return Err(RawCaptureAdmissionError {
                reason: state.reason.unwrap_or(CIRCUIT_REASON_INVENTORY_UNREADY),
            });
        }
        state.reserved_bytes = state.reserved_bytes.saturating_add(requested_bytes);
        Ok(RawCaptureReservation {
            circuit: self.clone(),
            reserved_bytes: requested_bytes,
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
        evaluate_locked(&mut state, requested_bytes, false);
        if state.state != CIRCUIT_STATE_CAPTURING {
            return Err(RawCaptureAdmissionError {
                reason: state.reason.unwrap_or(CIRCUIT_REASON_INVENTORY_UNREADY),
            });
        }
        state.reserved_bytes = state.reserved_bytes.saturating_add(requested_bytes);
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
        self.reserved_bytes = self.reserved_bytes.saturating_add(additional_bytes);
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

fn normalize_reason(reason: Option<&str>) -> Option<&'static str> {
    match reason {
        Some(CIRCUIT_REASON_RAW_STORE_LIMIT) => Some(CIRCUIT_REASON_RAW_STORE_LIMIT),
        Some(CIRCUIT_REASON_FILESYSTEM_LOW) => Some(CIRCUIT_REASON_FILESYSTEM_LOW),
        Some(CIRCUIT_REASON_BOTH) => Some(CIRCUIT_REASON_BOTH),
        Some(CIRCUIT_REASON_INVENTORY_UNREADY) => Some(CIRCUIT_REASON_INVENTORY_UNREADY),
        _ => None,
    }
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
            } else {
                CIRCUIT_REASON_FILESYSTEM_LOW
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
            Some(available_bytes),
            Some(0),
            Some(backlog_non_growing),
            None,
        );
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
            RAW_CAPTURE_CLOSE_BYTES - 10,
            RAW_CAPTURE_RESUME_AVAILABLE_BYTES,
            true,
        );
        let _reservation = circuit.admit(9).expect("first reservation should fit");
        let error = circuit
            .admit(2)
            .expect_err("second reservation crosses close watermark");
        assert_eq!(error.reason, CIRCUIT_REASON_RAW_STORE_LIMIT);
    }
}
