use super::*;

mod archive_identity;
pub(super) mod batch_plan;
mod observability;
mod task_batches;
mod task_transaction_resources;
use archive_identity::invocation_archive_source_identity_matches;
pub(crate) use archive_identity::{
    InvocationArchiveIdentityDatabase, archive_table_source_identity_sha256,
    invocation_archive_source_identity_sha256,
};
#[cfg(test)]
pub(crate) use archive_identity::{
    invocation_archive_source_identity_matches_for_test,
    invocation_archive_source_identity_sha256_candidate_v2_for_test,
    invocation_archive_source_identity_sha256_legacy_for_test,
};
pub(crate) use batch_plan::archive_file_can_start;
#[cfg(test)]
pub(crate) use batch_plan::{
    BatchObservation, archive_sqlite_size_from_header, retention_test_with_batch_metrics,
};
pub(crate) use batch_plan::{
    RetentionBatchMetrics, TaskArchiveSnapshotPage, TaskArchiveSnapshotPageFingerprint,
    archive_source_row_sizes, clear_task_summary_snapshot_pages,
    stage_task_summary_archive_manifest, store_task_summary_snapshot_page,
};
use batch_plan::{archive_batch_can_start, archive_candidate_limit, select_archive_batch};
pub(crate) use observability::record_backlog_observation;
use task_batches::archive_old_invocations_with_source_max;
#[cfg(test)]
pub(crate) use task_batches::{
    archive_old_invocations, invocation_archive_candidate_selection_for_test,
};
pub(crate) use task_batches::{archive_timestamped_dataset, prune_old_invocation_details};
pub(crate) use task_transaction_resources::RetentionTransactionResources;
#[cfg(test)]
pub(crate) use task_transaction_resources::{
    RETENTION_CLEANUP_WAITER_READY, RETENTION_TEST_RETURN_TO_POOL, RetentionReturnToPoolProbe,
};

use chrono::{TimeZone, Timelike};
use chrono_tz::Asia::Shanghai;
#[cfg(test)]
use tokio::sync::Notify;
#[path = "retention/workload.rs"]
mod workload;
use sqlx::FromRow;
use std::{
    cell::RefCell,
    collections::VecDeque,
    fs::{File, ReadDir},
    future::Future,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
mod archive_candidates;
mod archive_models;
#[path = "retention/batch_admission.rs"]
mod batch_admission;
mod legacy_archive_recovery;
mod maintenance_loop;
mod prepared_archive_recovery;
mod raw_compression;
mod raw_orphan_sweep;
mod raw_reconciliation;
mod write_admission;
pub(crate) use batch_admission::acquire_retention_batch_write_connection;
#[cfg(test)]
pub(crate) use batch_admission::{
    ArchiveArtifactProbe, ArchiveFileVerifyProbe, ArchiveIdentityProofProbe, BatchCommitProbe,
    GateGenerationProbe, RETENTION_TEST_ARCHIVE_ARTIFACT, RETENTION_TEST_ARCHIVE_FILE_VERIFY,
    RETENTION_TEST_ARCHIVE_IDENTITY_PROOF, RETENTION_TEST_BATCH_COMMIT,
    RETENTION_TEST_GATE_GENERATION, RETENTION_TEST_TRANSACTION_BEGIN, TransactionBeginProbe,
};

pub(crate) use workload::run_data_retention_maintenance_with_circuit_and_prompt_cache;

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

pub(crate) use archive_candidates::*;
pub(crate) use archive_models::*;
pub(crate) use legacy_archive_recovery::*;
pub(crate) use maintenance_loop::*;
pub(crate) use prepared_archive_recovery::*;
pub(crate) use raw_compression::*;
pub(crate) use raw_orphan_sweep::*;
pub(crate) use raw_reconciliation::*;
pub(crate) use write_admission::*;

#[cfg(test)]
mod ttft_retention_tests {
    use super::*;

    #[test]
    fn archived_invocation_keeps_ttft_for_rollup_materialization() {
        let candidate = InvocationArchiveCandidate {
            id: 7,
            invoke_id: "invoke-7".to_string(),
            occurred_at: "2026-07-25 12:00:00".to_string(),
            source: SOURCE_PROXY.to_string(),
            model: Some("gpt-5-codex".to_string()),
            status: Some("success".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(2),
            cache_input_tokens: Some(3),
            reasoning_tokens: None,
            total_tokens: Some(12),
            cost: Some(0.01),
            cost_input: Some(0.005),
            cost_cache_write: None,
            cost_cache_read: Some(0.001),
            cost_output: Some(0.004),
            cost_reasoning: None,
            error_message: None,
            failure_kind: None,
            failure_class: None,
            is_actionable: Some(1),
            first_token_ms: Some(321.0),
            payload: None,
            request_raw_path: None,
            response_raw_path: None,
        };

        let row = invocation_archive_candidate_to_hourly_source_record(&candidate);

        assert_eq!(row.first_token_ms, Some(321.0));
        assert_eq!(row.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(row.cost_input, Some(0.005));
        assert_eq!(row.cost_cache_read, Some(0.001));
        assert_eq!(row.cost_output, Some(0.004));
        assert_eq!(row.is_actionable, Some(1));
    }
}

#[cfg(test)]
mod retention_write_budget_tests {
    use super::*;

    #[test]
    fn raw_path_ledger_aliases_normalize_relative_database_roots() {
        let aliases = raw_path_ledger_aliases(
            "proxy_raw_payloads/sample.bin",
            Some(Path::new("relative-database")),
        );
        let absolute_root =
            std::path::absolute("relative-database").expect("resolve relative database root");
        let absolute_path = absolute_root.join("proxy_raw_payloads/sample.bin");
        let absolute_compressed_path = absolute_root.join("proxy_raw_payloads/sample.bin.gz");
        let cwd_relative_path =
            std::path::absolute("relative-database/proxy_raw_payloads/sample.bin")
                .expect("resolve cwd-relative raw path");

        assert!(aliases.contains(&absolute_path.to_string_lossy().into_owned()));
        assert!(aliases.contains(&absolute_compressed_path.to_string_lossy().into_owned()));
        assert!(aliases.contains(&cwd_relative_path.to_string_lossy().into_owned()));

        let prefixed_aliases = raw_path_ledger_aliases(
            "relative-database/proxy_raw_payloads/sample.bin",
            Some(Path::new("relative-database")),
        );
        assert!(prefixed_aliases.contains(&"proxy_raw_payloads/sample.bin".to_string()));
        assert!(prefixed_aliases.contains(&"proxy_raw_payloads/sample.bin.gz".to_string()));

        let absolute_prefixed_path =
            std::path::absolute("relative-database/proxy_raw_payloads/sample.bin")
                .expect("resolve absolute prefixed raw path");
        let absolute_prefixed_aliases = raw_path_ledger_aliases(
            &absolute_prefixed_path.to_string_lossy(),
            Some(Path::new("relative-database")),
        );
        assert!(
            absolute_prefixed_aliases
                .contains(&"relative-database/proxy_raw_payloads/sample.bin".to_string())
        );
        assert!(
            absolute_prefixed_aliases
                .contains(&"relative-database/proxy_raw_payloads/sample.bin.gz".to_string())
        );

        let absolute_fallback_prefixed_aliases = raw_path_ledger_aliases(
            "relative-database/proxy_raw_payloads/sample.bin",
            Some(&absolute_root),
        );
        assert!(
            absolute_fallback_prefixed_aliases
                .contains(&"proxy_raw_payloads/sample.bin".to_string())
        );
        assert!(
            absolute_fallback_prefixed_aliases
                .contains(&"proxy_raw_payloads/sample.bin.gz".to_string())
        );
        assert!(
            absolute_fallback_prefixed_aliases
                .contains(&absolute_path.to_string_lossy().into_owned())
        );

        let absolute_fallback_unprefixed_aliases =
            raw_path_ledger_aliases("proxy_raw_payloads/sample.bin", Some(&absolute_root));
        assert!(
            absolute_fallback_unprefixed_aliases
                .contains(&absolute_path.to_string_lossy().into_owned())
        );
        assert!(
            absolute_fallback_unprefixed_aliases
                .contains(&"relative-database/proxy_raw_payloads/sample.bin".to_string())
        );
    }

    #[test]
    fn retention_write_budget_adapts_without_exceeding_hard_bounds() {
        let mut budget = RetentionWriteBudget::default();
        assert_eq!(budget.candidate_limit(1_000), RETENTION_WRITE_INITIAL_ROWS);

        assert!(budget.observe_commit(4, 4 * 256, Duration::from_millis(251)));
        assert_eq!(budget.candidate_limit(1_000), 2);

        assert!(budget.observe_commit(1, RETENTION_WRITE_MAX_BYTES + 1, Duration::ZERO));
        assert_eq!(budget.candidate_limit(1_000), 1);

        assert!(!budget.observe_commit(1, 256, Duration::from_millis(1)));
        assert_eq!(
            budget.candidate_limit(1_000),
            2,
            "a normal commit must recover the candidate from an oversized-row estimate"
        );
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
            OPERATION,
            "normal",
            4,
            4 * 256,
            Duration::ZERO,
            Duration::ZERO,
            Duration::from_millis(251),
            Duration::ZERO,
            None,
            0,
            0,
        ));
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            2
        );
        assert_eq!(health.snapshot.state, "degraded");
    }

    #[test]
    fn retention_write_budget_uses_actual_write_hold_duration() {
        const OPERATION: &str = "retention_test_write_hold_budget";
        let mut health = RetentionWriteHealthState::default();
        assert!(!observe_retention_write_commit_with_hold(
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
                write_hold_elapsed: Some(Duration::from_millis(1)),
                raw_reference_check_elapsed: None,
                p1_waiter_count: 0,
                candidate_remaining_hint: 0,
            },
        ));
        assert_eq!(
            retention_adaptive_candidate_limit_from_state(&mut health, 64, OPERATION),
            5
        );
        assert_eq!(health.snapshot.state, "healthy");
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

#[cfg(test)]
mod retention_recovery_race_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn quarantine_guard_does_not_overwrite_a_republished_artifact() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory retention database");
        sqlx::query(
            r#"
            CREATE TABLE retention_prepared_archives (
                prepared_key TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                artifact_sha256 TEXT,
                quarantined_at TEXT,
                last_failure_stage TEXT,
                last_failure_fingerprint TEXT,
                updated_at TEXT
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("create prepared archive fixture");
        sqlx::query(
            "INSERT INTO retention_prepared_archives (prepared_key, state, artifact_sha256) VALUES ('race', 'published', 'old-sha')",
        )
        .execute(&pool)
        .await
        .expect("seed published archive fixture");

        sqlx::query(
            "UPDATE retention_prepared_archives SET state = 'published', artifact_sha256 = 'new-sha' WHERE prepared_key = 'race'",
        )
        .execute(&pool)
        .await
        .expect("republish archive fixture before stale quarantine update");

        let updated = quarantine_published_retention_archive_if_unchanged(
            &pool,
            "race",
            Some("old-sha"),
            Some("fingerprint"),
        )
        .await
        .expect("run stale quarantine guard");
        assert!(!updated);

        let row: (String, String) = sqlx::query_as(
            "SELECT state, artifact_sha256 FROM retention_prepared_archives WHERE prepared_key = 'race'",
        )
        .fetch_one(&pool)
        .await
        .expect("load republished archive fixture");
        assert_eq!(row, ("published".to_string(), "new-sha".to_string()));
    }

    #[test]
    fn staged_legacy_archive_restore_replaces_interrupted_artifact() {
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-restore-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create staged restore directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"replacement artifact").expect("write replacement artifact");
        fs::write(&staged_path, b"previous artifact").expect("write previous artifact");
        let expected_sha = sha256_hex_file(&staged_path).expect("hash previous artifact");

        restore_staged_legacy_archive_file(&staged_path, &final_path, &expected_sha)
            .expect("restore staged legacy archive");
        assert_eq!(
            fs::read(&final_path).expect("read restored archive"),
            b"previous artifact"
        );
        assert!(!staged_path.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn staged_archive_replacement_reconcile_restores_manifest_old_bytes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect staged replacement database");
        sqlx::query(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                replacement_staged_path TEXT,
                sha256 TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create staged replacement manifest");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-reconcile-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create staged reconcile directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"replacement artifact").expect("write replacement artifact");
        fs::write(&staged_path, b"previous artifact").expect("write previous artifact");
        let old_sha = sha256_hex_file(&staged_path).expect("hash previous artifact");
        let file_path = final_path.to_string_lossy().to_string();
        let staged_path_string = staged_path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, replacement_staged_path, sha256) VALUES (1, ?1, ?2, ?3)",
        )
        .bind(&file_path)
        .bind(&staged_path_string)
        .bind(&old_sha)
        .execute(&pool)
        .await
        .expect("seed staged replacement manifest");

        reconcile_staged_archive_replacements(&pool, None)
            .await
            .expect("reconcile staged replacement");
        assert_eq!(
            fs::read(&final_path).expect("read restored manifest file"),
            b"previous artifact"
        );
        let staged: Option<String> =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load cleared replacement staging path");
        assert!(staged.is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn staged_archive_replacement_reconcile_keeps_unverified_rollback_pending() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect unverified staged replacement database");
        sqlx::query(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                replacement_staged_path TEXT,
                sha256 TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create unverified staged replacement manifest");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-unverified-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create unverified staged directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"replacement artifact").expect("write replacement artifact");
        fs::write(&staged_path, b"corrupt rollback artifact").expect("write corrupt rollback");
        let expected_old_path = root.join("expected-old-bytes");
        fs::write(&expected_old_path, b"previous artifact").expect("write expected old bytes");
        let old_sha = sha256_hex_file(&expected_old_path).expect("hash expected old bytes");
        fs::remove_file(&expected_old_path).expect("remove expected old bytes");
        let file_path = final_path.to_string_lossy().to_string();
        let staged_path_string = staged_path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, replacement_staged_path, sha256) VALUES (1, ?1, ?2, ?3)",
        )
        .bind(&file_path)
        .bind(&staged_path_string)
        .bind(&old_sha)
        .execute(&pool)
        .await
        .expect("seed corrupt staged replacement manifest");

        reconcile_staged_archive_replacements(&pool, None)
            .await
            .expect("reconcile should retain unverified staged replacement");
        assert_eq!(
            fs::read(&final_path).expect("read unchanged replacement artifact"),
            b"replacement artifact"
        );
        let staged: Option<String> =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load retained replacement staging path");
        assert_eq!(staged.as_deref(), Some(staged_path_string.as_str()));
        let _ = fs::remove_dir_all(&root);
    }

    #[tokio::test]
    async fn staged_archive_replacement_reconcile_does_not_decode_or_delete_null_digest() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect null digest replacement database");
        sqlx::query(
            "CREATE TABLE archive_batches (
                id INTEGER PRIMARY KEY,
                file_path TEXT NOT NULL,
                replacement_staged_path TEXT,
                sha256 TEXT
            )",
        )
        .execute(&pool)
        .await
        .expect("create null digest replacement manifest");
        let root = std::env::temp_dir().join(format!(
            "codex-vibe-monitor-staged-null-{}-{}",
            std::process::id(),
            retention_temp_suffix()
        ));
        fs::create_dir_all(&root).expect("create null digest staged directory");
        let final_path = root.join("archive.sqlite.gz");
        let staged_path = root.join("archive.sqlite.gz.restore");
        fs::write(&final_path, b"current artifact").expect("write current artifact");
        fs::write(&staged_path, b"unknown rollback artifact").expect("write unknown rollback");
        let file_path = final_path.to_string_lossy().to_string();
        let staged_path_string = staged_path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO archive_batches (id, file_path, replacement_staged_path, sha256) VALUES (1, ?1, ?2, NULL)",
        )
        .bind(&file_path)
        .bind(&staged_path_string)
        .execute(&pool)
        .await
        .expect("seed null digest replacement manifest");

        reconcile_staged_archive_replacements(&pool, None)
            .await
            .expect("reconcile should retain null digest replacement");
        assert_eq!(
            fs::read(&final_path).expect("read unchanged current artifact"),
            b"current artifact"
        );
        assert!(staged_path.exists());
        let staged: Option<String> =
            sqlx::query_scalar("SELECT replacement_staged_path FROM archive_batches WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("load retained null digest staging path");
        assert_eq!(staged.as_deref(), Some(staged_path_string.as_str()));
        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
#[path = "../tests/lightweight/retention_observability.rs"]
mod retention_observability;

#[cfg(test)]
mod retention_summary_tests {
    use chrono::{TimeZone, Utc};

    use super::{RetentionRunSummary, retention_backlog_max_overdue_seconds};

    #[test]
    fn selected_batch_completion_does_not_complete_captured_backlog() {
        let mut summary = RetentionRunSummary {
            backlog_total: Some(1_001),
            invocation_rows_archived: 1_000,
            ..RetentionRunSummary::default()
        };
        assert_eq!(summary.completion(), "partial");
        assert_eq!(summary.core_completion(), "completed");
        summary.invocation_rows_archived = 1_001;
        assert_eq!(summary.completion(), "completed");
        summary.invocation_rows_archived = 0;
        assert_eq!(summary.completion(), "partial", "known pending work");
        summary.backlog_total = Some(0);
        assert_eq!(summary.completion(), "completed", "accurate empty scope");
    }

    #[test]
    fn pending_coverage_is_partial_but_pure_admission_is_deferred() {
        let mut summary = RetentionRunSummary {
            backlog_total: Some(1_001),
            deferred: true,
            wait_reason: Some("parallel_work_minute_coverage".to_string()),
            ..RetentionRunSummary::default()
        };
        assert_eq!(summary.completion(), "partial");
        for reason in [
            "sqlite_pressure",
            "sqlite_pool_wait",
            "retention_write_admission",
        ] {
            summary.wait_reason = Some(reason.to_string());
            assert_eq!(summary.completion(), "deferred", "pure admission: {reason}");
        }
        summary.invocation_rows_archived = 64;
        assert_eq!(
            summary.completion(),
            "partial",
            "committed progress remains"
        );
        summary.fatal_error = Some("archive failed".to_string());
        assert_eq!(summary.completion(), "failed", "fatal error takes priority");
    }

    #[test]
    fn recoverable_failure_is_partial_without_progress() {
        let summary = RetentionRunSummary {
            deferred: true,
            recoverable_failure: true,
            ..RetentionRunSummary::default()
        };
        assert_eq!(summary.completion(), "partial");
        assert_eq!(summary.core_completion(), "partial");
    }

    #[test]
    fn timeout_without_committed_progress_is_partial_not_pure_admission() {
        let mut summary = RetentionRunSummary {
            backlog_total: Some(1_001),
            budget_exhausted: true,
            wait_reason: Some("retention_work_budget".to_string()),
            ..RetentionRunSummary::default()
        };
        assert_eq!(summary.completion(), "partial");
        summary.deferred = true;
        assert_eq!(summary.completion(), "partial", "timeout wins over defer");
        summary.backlog_total = None;
        assert_eq!(summary.completion(), "partial", "unknown scope timed out");
        summary.fatal_error = Some("archive failed".to_string());
        assert_eq!(summary.completion(), "failed", "fatal error takes priority");
    }

    #[test]
    fn fatal_error_remains_failed_even_when_recoverable_state_is_present() {
        let summary = RetentionRunSummary {
            fatal_error: Some("archive failed".to_string()),
            recoverable_failure: true,
            ..RetentionRunSummary::default()
        };
        assert_eq!(summary.completion(), "failed");
        assert_eq!(summary.core_completion(), "failed");
    }

    #[test]
    fn backlog_overdue_uses_shanghai_natural_day_eligibility() {
        let observed_at = Utc
            .with_ymd_and_hms(2026, 10, 2, 0, 0, 0)
            .single()
            .expect("valid observation timestamp");
        assert_eq!(
            retention_backlog_max_overdue_seconds(Some("2026-09-24T16:00:00.000Z"), 7, observed_at,),
            Some(28_800)
        );
        assert_eq!(
            retention_backlog_max_overdue_seconds(Some("2026-09-24T15:59:59.000Z"), 7, observed_at,),
            Some(115_200)
        );
        assert_eq!(
            retention_backlog_max_overdue_seconds(Some("2026-09-25T16:00:00.000Z"), 7, observed_at,),
            Some(0)
        );
    }
}
