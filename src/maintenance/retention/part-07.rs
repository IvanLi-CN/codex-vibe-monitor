#[cfg(test)]
mod retention_write_budget_tests {
    use super::*;

    #[test]
    fn retention_write_budget_adapts_without_exceeding_hard_bounds() {
        let mut budget = RetentionWriteBudget::default();
        assert_eq!(budget.candidate_limit(1_000), RETENTION_WRITE_INITIAL_ROWS);

        assert!(budget.observe_commit(4, 4 * 256, Duration::from_millis(251)));
        assert_eq!(budget.candidate_limit(1_000), 2);

        assert!(budget.observe_commit(1, RETENTION_WRITE_MAX_BYTES + 1, Duration::ZERO));
        assert_eq!(budget.candidate_limit(1_000), 1);

        for _ in 0..100 {
            assert!(!budget.observe_commit(1, 256, Duration::from_millis(1)));
        }
        assert!(budget.candidate_limit(1_000) <= RETENTION_WRITE_MAX_ROWS);
    }

    #[test]
    fn retention_health_records_a_budget_breach_for_the_next_production_candidate() {
        const OPERATION: &str = "retention_test_adaptive_budget";
        let mut health = RetentionWriteHealthState::default();
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            4
        );
        assert!(observe_retention_write_commit(
            &mut health,
            &RetentionWriteCommit {
                operation: OPERATION,
                admission_mode: "normal",
                rows: 4,
                estimated_bytes: 4 * 256,
                prepare_elapsed: Duration::ZERO,
                lock_wait: Duration::ZERO,
                execute_elapsed: Duration::from_millis(251),
                commit_elapsed: Duration::ZERO,
                p1_waiter_count: 0,
                candidate_remaining_hint: 0,
            },
        ));
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            2
        );
        assert_eq!(health.snapshot.state, "degraded");
    }

    #[test]
    fn retention_micro_batch_keeps_a_single_oversized_row_losslessly() {
        let selected =
            take_retention_micro_batch(vec![2 * RETENTION_WRITE_MAX_BYTES, 128], |value| *value);
        assert_eq!(selected, vec![2 * RETENTION_WRITE_MAX_BYTES]);
    }

    #[test]
    fn system_task_run_retention_only_backs_off_for_pressure_cooldown() {
        assert!(
            system_task_run_retention_admission_requires_pressure_backoff(Some(
                crate::db_pressure::DbPressureDenyReason::PressureCooldown { remaining_ms: 1 }
            ))
        );
        assert!(
            !system_task_run_retention_admission_requires_pressure_backoff(Some(
                crate::db_pressure::DbPressureDenyReason::BackgroundBusy
            ))
        );
        assert!(!system_task_run_retention_admission_requires_pressure_backoff(None));
    }
}

#[derive(Debug, FromRow)]
pub(crate) struct ArchiveExpiryBackfillCandidate {
    pub(crate) id: i64,
    pub(crate) coverage_end_at: String,
}
