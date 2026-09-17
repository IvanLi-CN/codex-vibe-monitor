#[cfg(test)]
mod tests {
    use super::*;

    fn test_guard() -> tokio::sync::OwnedMutexGuard<()> {
        priority_handoff_test_guard_blocking()
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

    #[test]
    fn priority_handoff_manual_reset_restarts_verification() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        for _ in 0..3 {
            let (_, permit) = admit_priority_handoff(9_012, Some("gpt-test"));
            permit.expect("permit").complete_success();
        }
        assert_eq!(
            priority_handoff_admission_snapshot(9_012, Some("gpt-test")),
            ("open".to_string(), 3)
        );

        reset_priority_handoff_for_model(9_012, "gpt-test");

        assert_eq!(
            priority_handoff_admission_snapshot(9_012, Some("gpt-test")),
            ("verifying".to_string(), 0)
        );
        let (_, permit) = admit_priority_handoff(9_012, Some("gpt-test"));
        let permit = permit.expect("post-reset permit");
        assert_eq!(
            permit.complete_success(),
            Some(PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON)
        );
        assert_eq!(
            priority_handoff_admission_snapshot(9_012, Some("gpt-test")),
            ("verifying".to_string(), 1)
        );
    }

    #[test]
    fn priority_handoff_manual_reset_fences_in_flight_permit() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        let (_, old_permit) = admit_priority_handoff(9_013, Some("gpt-test"));
        let old_permit = old_permit.expect("old permit");

        reset_priority_handoff_for_model(9_013, "gpt-test");

        let (blocked_decision, new_permit) = admit_priority_handoff(9_013, Some("gpt-test"));
        assert_eq!(
            blocked_decision,
            PriorityHandoffAdmissionDecision::PermitBusy
        );
        assert!(new_permit.is_none());
        assert!(old_permit.complete_success().is_none());
        drop(old_permit);
        let (_, new_permit) = admit_priority_handoff(9_013, Some("gpt-test"));
        let new_permit = new_permit.expect("post-reset permit");
        assert_eq!(
            new_permit.complete_success(),
            Some(PRIORITY_HANDOFF_RECOVERY_PROGRESS_REASON)
        );
        assert_eq!(
            priority_handoff_admission_snapshot(9_013, Some("gpt-test")),
            ("verifying".to_string(), 1)
        );
        drop(new_permit);
    }

    #[test]
    fn request_driven_priority_recovery_keeps_one_permit_and_restarts_at_zero() {
        let _guard = test_guard();
        set_priority_handoff_admission_enabled(true);
        restart_priority_handoff_verification_for_model(9_016, "gpt-5.6-terra");

        let (decision, old_permit) = admit_priority_handoff(9_016, Some("gpt-5.6-terra"));
        assert!(matches!(
            decision,
            PriorityHandoffAdmissionDecision::Admitted { .. }
        ));
        let old_permit = old_permit.expect("first recovery permit");
        let (busy, _) = admit_priority_handoff(9_016, Some("gpt-5.6-terra"));
        assert_eq!(busy, PriorityHandoffAdmissionDecision::PermitBusy);

        restart_priority_handoff_verification_for_model(9_016, "gpt-5.6-terra");
        assert_eq!(
            priority_handoff_admission_snapshot(9_016, Some("gpt-5.6-terra")),
            ("verifying".to_string(), 0)
        );
        assert!(old_permit.complete_success().is_none());

        let (next, new_permit) = admit_priority_handoff(9_016, Some("gpt-5.6-terra"));
        assert!(matches!(
            next,
            PriorityHandoffAdmissionDecision::Admitted { .. }
        ));
        drop(new_permit);
    }

    #[test]
    fn routing_handoff_audit_compatibility_accepts_optional_trigger() {
        let old = serde_json::json!({
            "selectedAccountId": 11,
            "selectedAccountName": "Aster",
            "eligibleCandidateCount": 1,
            "winnerReasonCode": "onlyEligibleCandidate",
            "handoffAdmission": {
                "decision": "admitted",
                "phase": "verifying",
                "verificationSuccessCount": 0,
                "generation": 3
            },
            "excludedCandidates": []
        });
        let old_audit: PoolRoutingSelectionAudit =
            serde_json::from_value(old).expect("historical audit remains readable");
        assert_eq!(old_audit.handoff_admission.unwrap().trigger, None);

        let current = serde_json::json!({
            "selectedAccountId": 2918,
            "selectedAccountName": "Ciii2",
            "eligibleCandidateCount": 2,
            "winnerReasonCode": "requestDrivenRecoveryAdmission",
            "handoffAdmission": {
                "decision": "admitted",
                "phase": "verifying",
                "verificationSuccessCount": 1,
                "generation": 8,
                "trigger": "modelRouteRecovery"
            },
            "excludedCandidates": []
        });
        let current_audit: PoolRoutingSelectionAudit =
            serde_json::from_value(current).expect("current audit remains readable");
        assert_eq!(
            current_audit.handoff_admission.unwrap().trigger.as_deref(),
            Some("modelRouteRecovery")
        );
    }

    #[tokio::test]
    async fn priority_handoff_recovery_generation_releases_fenced_success_without_counting() {
        let _guard = priority_handoff_test_guard().await;
        set_priority_handoff_admission_enabled(true);
        let (decision, permit) = admit_priority_handoff(9_017, Some("gpt-5.6-terra"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected recovery admission");
        };
        let audit_json = serde_json::json!({
            "selectedAccountId": 9_017,
            "selectedAccountName": "Ciii2",
            "eligibleCandidateCount": 2,
            "winnerReasonCode": "requestDrivenRecoveryAdmission",
            "handoffAdmission": {
                "decision": "admitted",
                "phase": "verifying",
                "verificationSuccessCount": 0,
                "generation": generation,
                "trigger": "modelRouteRecovery"
            },
            "excludedCandidates": []
        })
        .to_string();
        remember_priority_handoff_attempt(
            None,
            Some("fenced-success"),
            9_017,
            Some("gpt-5.6-terra"),
            Some(audit_json.as_str()),
        );
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("in-memory sqlite pool");

        complete_priority_handoff_from_attempt_or_invoke_with_model_recovery(
            &pool,
            None,
            Some("fenced-success"),
            true,
            false,
            Some(false),
        )
        .await;

        assert_eq!(
            priority_handoff_admission_snapshot(9_017, Some("gpt-5.6-terra")),
            ("verifying".to_string(), 0)
        );
        let (_, next) = admit_priority_handoff(9_017, Some("gpt-5.6-terra"));
        assert!(next.is_some());
        drop(permit);
        drop(next);
    }

    #[tokio::test]
    async fn priority_handoff_database_failure_does_not_block_local_transition() {
        let _guard = priority_handoff_test_guard().await;
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
            None,
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
    async fn priority_handoff_missing_attempt_id_keeps_local_failure_transition() {
        let _guard = priority_handoff_test_guard().await;
        set_priority_handoff_admission_enabled(true);
        let (decision, permit) = admit_priority_handoff(9_015, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected admission");
        };
        let audit_json = serde_json::json!({
            "selectedAccountId": 9_015,
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
            None,
            Some("invoke-without-persisted-attempt"),
            9_015,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("in-memory sqlite pool");
        complete_priority_handoff_from_attempt_or_invoke(
            &pool,
            None,
            Some("invoke-without-persisted-attempt"),
            false,
            true,
        )
        .await;
        drop(permit);
        assert_eq!(
            priority_handoff_admission_snapshot(9_015, Some("gpt-test")).0,
            "coolingDown"
        );
    }

    #[tokio::test]
    async fn priority_handoff_finalized_success_advances_from_attempt_record() {
        let _guard = priority_handoff_test_guard().await;
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
            None,
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

    fn test_handoff_audit(account_id: i64, generation: u64) -> String {
        serde_json::json!({
            "selectedAccountId": account_id,
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
        .to_string()
    }

    async fn insert_unknown_terminal_attempt(pool: &sqlx::SqlitePool, id: i64, status: &str) {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, status, downstream_http_status, failure_kind) VALUES (?1, ?2, NULL, NULL)",
        )
        .bind(id)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert unknown terminal attempt");
    }

    async fn assert_unknown_terminal_releases_first_attempt(
        pool: &sqlx::SqlitePool,
        decision: PriorityHandoffAdmissionDecision,
        permit: Option<Arc<PriorityHandoffPermit>>,
    ) {
        let generation = match decision {
            PriorityHandoffAdmissionDecision::Admitted { generation } => generation,
            _ => unreachable!(),
        };
        let audit_json = test_handoff_audit(9_009, generation);
        remember_priority_handoff_attempt(
            Some(70_009),
            None,
            9_009,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );
        complete_priority_handoff_from_attempt(pool, Some(70_009), true, false).await;
        let (_, next) = admit_priority_handoff(9_009, Some("gpt-test"));
        assert!(next.is_some());
        drop(permit);
        drop(next);
    }

    async fn assert_pending_terminal_keeps_admission_busy(pool: &sqlx::SqlitePool) {
        let (decision, permit) = admit_priority_handoff(9_010, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected pending admission");
        };
        insert_unknown_terminal_attempt(pool, 70_010, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_PENDING)
            .await;
        let audit_json = test_handoff_audit(9_010, generation);
        remember_priority_handoff_attempt(
            Some(70_010),
            None,
            9_010,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );
        complete_priority_handoff_from_attempt(pool, Some(70_010), true, false).await;
        let (_, blocked) = admit_priority_handoff(9_010, Some("gpt-test"));
        assert!(blocked.is_none());
        complete_priority_handoff_from_attempt(pool, Some(70_010), false, false).await;
        drop(permit);
    }

    async fn assert_explicit_unknown_terminal_releases_attempt(pool: &sqlx::SqlitePool) {
        let (decision, permit) = admit_priority_handoff(9_011, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected explicit unknown admission");
        };
        insert_unknown_terminal_attempt(pool, 70_011, "aborted").await;
        let audit_json = test_handoff_audit(9_011, generation);
        remember_priority_handoff_attempt(
            Some(70_011),
            None,
            9_011,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );
        complete_priority_handoff_from_attempt(pool, Some(70_011), true, false).await;
        let (_, next) = admit_priority_handoff(9_011, Some("gpt-test"));
        assert!(next.is_some());
        drop(permit);
        drop(next);
    }

    #[tokio::test]
    async fn priority_handoff_unknown_terminal_status_releases_attempt() {
        let _guard = priority_handoff_test_guard().await;
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
            "INSERT INTO pool_upstream_request_attempts (id, status, downstream_http_status, failure_kind) VALUES (?1, ?2, NULL, NULL)",
        )
        .bind(70_009_i64)
        .bind(None::<&str>)
        .execute(&pool)
        .await
        .expect("insert unknown terminal attempt");
        assert_unknown_terminal_releases_first_attempt(&pool, decision, permit).await;

        assert_pending_terminal_keeps_admission_busy(&pool).await;

        assert_explicit_unknown_terminal_releases_attempt(&pool).await;
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
        assert!(new_generation > bumped_generation);

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
        let (still_busy, still_in_flight) = admit_priority_handoff(9_005, Some("gpt-test"));
        assert_eq!(still_busy, PriorityHandoffAdmissionDecision::PermitBusy);
        assert!(still_in_flight.is_none());
        drop(new_permit);
    }

    #[tokio::test]
    async fn priority_handoff_success_db_read_failure_releases_without_fabricating_success() {
        let _guard = priority_handoff_test_guard().await;
        set_priority_handoff_admission_enabled(true);
        let (decision, permit) = admit_priority_handoff(9_014, Some("gpt-test"));
        let PriorityHandoffAdmissionDecision::Admitted { generation } = decision else {
            panic!("expected admission");
        };
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .expect("in-memory sqlite pool");
        let audit_json = serde_json::json!({
            "selectedAccountId": 9_014,
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
            Some(70_014),
            None,
            9_014,
            Some("gpt-test"),
            Some(audit_json.as_str()),
        );

        complete_priority_handoff_from_attempt(&pool, Some(70_014), true, false).await;

        let (next_decision, next_permit) = admit_priority_handoff(9_014, Some("gpt-test"));
        assert!(matches!(
            next_decision,
            PriorityHandoffAdmissionDecision::Admitted { .. }
        ));
        assert!(next_permit.is_some());
        assert_eq!(
            priority_handoff_admission_snapshot(9_014, Some("gpt-test")),
            ("verifying".to_string(), 0)
        );
        drop(next_permit);
        drop(permit);
    }
}
