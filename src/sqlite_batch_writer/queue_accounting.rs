use super::*;

impl PendingQueueAccounting {
    pub(super) fn attempt_progress_enqueued(&self) {
        self.attempt_progress_enqueued
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn attempt_progress_coalesced(&self, count: usize) {
        self.attempt_progress_coalesced
            .fetch_add(count.min(u64::MAX as usize) as u64, Ordering::Relaxed);
    }

    pub(super) fn attempt_progress_dropped(&self) {
        self.attempt_progress_dropped
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn attempt_progress_deferred(&self, count: usize) {
        self.attempt_progress_deferred
            .fetch_add(count.min(u64::MAX as usize) as u64, Ordering::Relaxed);
    }

    pub(super) fn observe_attempt_progress_age(&self, age_ms: u64) {
        let _ = self.attempt_progress_oldest_age_ms.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.max(age_ms)),
        );
    }

    pub(super) fn observe_p1_replace(&self, old: usize, new: usize) {
        if self.observability.get().is_some_and(|m| m.enabled) {
            let _ = self.observed_p1_depth.fetch_update(
                Ordering::Relaxed,
                Ordering::Relaxed,
                |value| Some(value.saturating_sub(old).saturating_add(new)),
            );
        }
    }

    pub(crate) fn observed_queue_depths(&self) -> (usize, usize) {
        let all = self.pending_depth.load(Ordering::Relaxed);
        let p1 = self.observed_p1_depth.load(Ordering::Relaxed).min(all);
        (p1, all.saturating_sub(p1))
    }

    pub(crate) fn enqueue(&self, bytes: usize) {
        self.add(&self.pending_bytes, bytes, "enqueue", "pending_bytes");
        self.add(&self.pending_depth, 1, "enqueue", "pending_depth");
    }

    pub(crate) fn rollback_enqueue(&self, bytes: usize) {
        self.subtract(
            &self.pending_depth,
            1,
            "sender_failure_rollback",
            "pending_depth",
        );
        self.subtract(
            &self.pending_bytes,
            bytes,
            "sender_failure_rollback",
            "pending_bytes",
        );
    }

    pub(crate) fn replace_batch(
        &self,
        admitted_depth: usize,
        retained_depth: usize,
        admitted_bytes: usize,
        retained_bytes: usize,
    ) {
        self.replace_for(
            "batch_replacement",
            admitted_depth,
            retained_depth,
            admitted_bytes,
            retained_bytes,
        );
    }

    pub(crate) fn transfer_p1_to_p2(&self, bytes: usize) {
        if let Some(m) = self.observability.get() {
            m.record_counter("p1.transfer_bytes", "p1", bytes as u64);
        }
        self.add(
            &self.transfer_bytes,
            bytes,
            "p1_to_p2_transfer",
            "transfer_bytes",
        );
    }

    pub(crate) fn record_p1_ack_duration(&self, duration_ms: u64) {
        if let Some(m) = self.observability.get() {
            m.record_duration_ms("p1.ack_duration_ms", "p1", duration_ms as f64);
        }
        self.p1_ack_duration_ms
            .store(duration_ms, Ordering::Relaxed);
        self.p1_ack_sequence.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_write_batch(&self, rows: usize, bytes: usize, duration_ms: u64) {
        self.write_batch_count.fetch_add(1, Ordering::Relaxed);
        self.write_rows
            .fetch_add(rows.min(u64::MAX as usize) as u64, Ordering::Relaxed);
        self.write_bytes
            .fetch_add(bytes.min(u64::MAX as usize) as u64, Ordering::Relaxed);
        self.write_duration_ms.store(duration_ms, Ordering::Relaxed);
    }

    pub(crate) fn retry_deferred(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn observe_p1_retry(&self) {
        if let Some(m) = self.observability.get() {
            m.record_counter("p1.retry_count", "p1", 1);
        }
    }

    pub(crate) fn retry_p1_deferred(&self) {
        self.retry_deferred();
        self.p1_retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn retry_p2_deferred(&self) {
        self.retry_deferred();
        self.p2_retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn p2_attempted(&self) {
        if let Some(m) = self.observability.get() {
            m.record_counter("p2.flush_attempt_count", "p2", 1);
        }
        self.p2_flush_attempt_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn p2_pressure_deferred(&self) {
        if let Some(m) = self.observability.get() {
            m.record_counter("p2.pressure_defer_count", "p2", 1);
        }
        self.p2_pressure_defer_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn observe_p2_retry(&self, lock_failure: bool) {
        if let Some(m) = self.observability.get() {
            let id = if lock_failure {
                "p2.lock_retry_count"
            } else {
                "p2.retry_count"
            };
            m.record_counter(id, "p2", 1);
        }
    }

    pub(super) fn p2_lock_retried(&self) {
        self.p2_lock_retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn update_p2_schedule(&self, schedule: &P2ScheduleState) {
        self.p2_next_attempt_in_ms
            .store(schedule.next_attempt_in_ms(), Ordering::Relaxed);
        self.p2_deferred_age_ms
            .store(schedule.deferred_age_ms(), Ordering::Relaxed);
        if let Ok(mut wake_reason) = self.p2_wake_reason.lock() {
            *wake_reason = schedule
                .wake_reason
                .map(|reason| reason.as_str().to_string());
        }
    }

    pub(crate) fn complete(
        &self,
        submitted_depth: usize,
        retained_depth: usize,
        submitted_bytes: usize,
        retained_bytes: usize,
    ) {
        self.replace_for(
            "completion",
            submitted_depth,
            retained_depth,
            submitted_bytes,
            retained_bytes,
        );
    }

    pub(crate) fn release(&self, depth: usize, bytes: usize) {
        self.subtract(&self.pending_depth, depth, "release", "pending_depth");
        self.subtract(&self.pending_bytes, bytes, "release", "pending_bytes");
    }

    pub(super) fn clear_after_shutdown(&self) -> (usize, usize) {
        self.observed_p1_depth.store(0, Ordering::Relaxed);
        let pending_depth = self.pending_depth.swap(0, Ordering::SeqCst);
        let pending_bytes = self.pending_bytes.swap(0, Ordering::SeqCst);
        (pending_depth, pending_bytes)
    }

    pub(crate) fn snapshot(&self) -> PendingQueueAccountingSnapshot {
        let last_invariant_violation = self
            .last_invariant_violation
            .lock()
            .ok()
            .and_then(|violation| violation.clone());
        let invariant_violation_count = self.invariant_violation_count.load(Ordering::Relaxed);
        let degraded_reason = last_invariant_violation.as_ref().map(|violation| {
            format!(
                "{} {} invariant: expected {}, actual {}",
                violation.operation,
                violation.counter,
                violation.expected_value,
                violation.actual_value
            )
        });
        PendingQueueAccountingSnapshot {
            state: if invariant_violation_count == 0 {
                "healthy".to_string()
            } else {
                "degraded".to_string()
            },
            pending_depth: self.pending_depth.load(Ordering::Relaxed),
            pending_bytes: self.pending_bytes.load(Ordering::Relaxed),
            p1_ack_sequence: self.p1_ack_sequence.load(Ordering::Relaxed),
            p1_ack_duration_ms: self.p1_ack_duration_ms.load(Ordering::Relaxed),
            transfer_bytes: self.transfer_bytes.load(Ordering::Relaxed),
            retry_count: self.retry_count.load(Ordering::Relaxed),
            p1_retry_count: self.p1_retry_count.load(Ordering::Relaxed),
            p2_retry_count: self.p2_retry_count.load(Ordering::Relaxed),
            p2_flush_attempt_count: self.p2_flush_attempt_count.load(Ordering::Relaxed),
            p2_pressure_defer_count: self.p2_pressure_defer_count.load(Ordering::Relaxed),
            p2_lock_retry_count: self.p2_lock_retry_count.load(Ordering::Relaxed),
            p2_next_attempt_in_ms: self.p2_next_attempt_in_ms.load(Ordering::Relaxed),
            p2_deferred_age_ms: self.p2_deferred_age_ms.load(Ordering::Relaxed),
            write_batch_count: self.write_batch_count.load(Ordering::Relaxed),
            write_rows: self.write_rows.load(Ordering::Relaxed),
            write_bytes: self.write_bytes.load(Ordering::Relaxed),
            write_duration_ms: self.write_duration_ms.load(Ordering::Relaxed),
            attempt_progress_enqueued: self.attempt_progress_enqueued.load(Ordering::Relaxed),
            attempt_progress_coalesced: self.attempt_progress_coalesced.load(Ordering::Relaxed),
            attempt_progress_dropped: self.attempt_progress_dropped.load(Ordering::Relaxed),
            attempt_progress_deferred: self.attempt_progress_deferred.load(Ordering::Relaxed),
            attempt_progress_oldest_age_ms: self
                .attempt_progress_oldest_age_ms
                .load(Ordering::Relaxed),
            p2_wake_reason: self
                .p2_wake_reason
                .lock()
                .ok()
                .and_then(|value| value.clone()),
            invariant_violation_count,
            degraded_reason,
            last_invariant_violation,
        }
    }

    pub(super) fn replace_for(
        &self,
        operation: &'static str,
        old_depth: usize,
        new_depth: usize,
        old_bytes: usize,
        new_bytes: usize,
    ) {
        self.subtract(&self.pending_depth, old_depth, operation, "pending_depth");
        self.add(&self.pending_depth, new_depth, operation, "pending_depth");
        self.subtract(&self.pending_bytes, old_bytes, operation, "pending_bytes");
        self.add(&self.pending_bytes, new_bytes, operation, "pending_bytes");
    }

    pub(super) fn add(
        &self,
        counter: &AtomicUsize,
        amount: usize,
        operation: &'static str,
        counter_name: &'static str,
    ) {
        if amount == 0 {
            return;
        }
        let previous = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(amount))
            })
            .expect("accounting update always returns a value");
        if previous.checked_add(amount).is_none() {
            self.record_invariant(operation, counter_name, amount, previous);
        }
    }

    pub(super) fn subtract(
        &self,
        counter: &AtomicUsize,
        amount: usize,
        operation: &'static str,
        counter_name: &'static str,
    ) {
        if amount == 0 {
            return;
        }
        let previous = counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(amount))
            })
            .expect("accounting update always returns a value");
        if previous < amount {
            self.record_invariant(operation, counter_name, amount, previous);
        }
    }

    pub(super) fn record_invariant(
        &self,
        operation: &'static str,
        counter: &'static str,
        expected_value: usize,
        actual_value: usize,
    ) {
        let pending_depth = self.pending_depth.load(Ordering::Relaxed);
        let pending_bytes = self.pending_bytes.load(Ordering::Relaxed);
        let (expected_bytes, actual_bytes) = if counter == "pending_bytes" {
            (expected_value, actual_value)
        } else {
            (pending_bytes, pending_bytes)
        };
        let violation = PendingQueueInvariantViolation {
            operation: operation.to_string(),
            counter: counter.to_string(),
            expected_value,
            actual_value,
            expected_bytes,
            actual_bytes,
            pending_depth,
            pending_bytes,
        };
        let invariant_violation_count = self
            .invariant_violation_count
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if let Ok(mut last) = self.last_invariant_violation.lock() {
            *last = Some(violation.clone());
        }
        warn!(
            accounting_invariant = true,
            operation,
            counter,
            expected_value,
            actual_value,
            pending_depth,
            pending_bytes,
            invariant_violation_count,
            "sqlite pending queue accounting invariant violated"
        );
    }
}
