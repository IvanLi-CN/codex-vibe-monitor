use crate::storage::archive::*;
use sha2::{Digest, Sha256};
use std::path::Path;

fn digest(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn manifest() -> ArchiveManifest {
    ArchiveManifest {
        archive_batch_id: 7,
        dataset: "codex_invocations".to_owned(),
        month_key: "2026-10".to_owned(),
        day_key: "2026-10-26".to_owned(),
        part: 0,
        file_path: "2026/10/26/part-0.sqlite.gz".to_owned(),
        sha256: digest("archive"),
        source_identity_sha256: digest("source"),
        row_count: 2,
        layout: "segment_v1".to_owned(),
        codec: "gzip".to_owned(),
        cleanup_state: "delete_pending".to_owned(),
        coverage_start_at: Some("2026-10-26T00:00:00Z".to_owned()),
        coverage_end_at: Some("2026-10-26T01:00:00Z".to_owned()),
    }
}

fn proof(manifest: &ArchiveManifest) -> ArchiveProof {
    ArchiveProof {
        archive_batch_id: manifest.archive_batch_id,
        manifest_sha256: manifest.sha256.clone(),
        page_count: 1,
        row_count: manifest.row_count,
        coverage_start_at: manifest.coverage_start_at.clone(),
        coverage_end_at: manifest.coverage_end_at.clone(),
        semantic_sha256: digest("semantic"),
        verified_at_epoch_ms: 100,
    }
}

#[test]
fn publication_validates_manifest_and_proof_identity() {
    let manifest = manifest();
    manifest
        .validate(Path::new("/var/lib/codex/archive"))
        .unwrap();
    let mut active_manifest = manifest.clone();
    active_manifest.cleanup_state = "active".to_owned();
    active_manifest
        .validate(Path::new("/var/lib/codex/archive"))
        .unwrap();
    proof(&manifest).validate_for(&manifest).unwrap();

    let mut mismatched = proof(&manifest);
    mismatched.manifest_sha256 = digest("other");
    assert!(mismatched.validate_for(&manifest).is_err());
}

#[test]
fn rollup_and_raw_reference_checks_bind_to_file_identity() {
    let manifest = manifest();
    let mut marker = ArchiveRollupMarker {
        archive_batch_id: manifest.archive_batch_id,
        target: "hourly".to_owned(),
        bucket_start_epoch_ms: 100,
        source_identity_sha256: digest("other-source"),
        materialized_rows: manifest.row_count,
        complete: true,
    };
    assert!(marker.validate(&manifest).is_err());
    marker.source_identity_sha256 = manifest.source_identity_sha256.clone();
    marker.validate(&manifest).unwrap();

    let missing_identity = RawReferenceChange {
        archive_batch_id: manifest.archive_batch_id,
        raw_path: "2026/10/request.bin".to_owned(),
        owner_kind: "invocation".to_owned(),
        owner_id: 1,
        raw_role: "request".to_owned(),
        action: RawReferenceAction::Link,
        raw_identity_sha256: None,
    };
    assert!(
        missing_identity
            .validate(&manifest, Path::new("/var/lib/codex/raw"))
            .is_err()
    );

    let invalid_owner = RawReferenceChange {
        raw_identity_sha256: Some(digest("raw")),
        owner_kind: "archive".to_owned(),
        ..missing_identity
    };
    assert!(
        invalid_owner
            .validate(&manifest, Path::new("/var/lib/codex/raw"))
            .is_err()
    );
}

#[test]
fn archive_publication_validation_covers_node_health_and_replay_identity() {
    let mut manifest = manifest();
    manifest.dataset = "pool_upstream_request_attempts".to_owned();
    let operation = ArchivePublishOperation {
        proof: proof(&manifest),
        rollups: vec![ArchiveRollupMarker {
            archive_batch_id: manifest.archive_batch_id,
            target: "hourly".to_owned(),
            bucket_start_epoch_ms: 100,
            source_identity_sha256: manifest.source_identity_sha256.clone(),
            materialized_rows: manifest.row_count,
            complete: true,
        }],
        replay_markers: vec![
            ArchiveReplayMarker {
                target: "pool_upstream_node_health_archive".to_owned(),
                dataset: manifest.dataset.clone(),
                file_path: manifest.file_path.clone(),
                archive_sha256: manifest.sha256.clone(),
            },
            ArchiveReplayMarker {
                target: "pool_upstream_node_health_hourly_archive".to_owned(),
                dataset: manifest.dataset.clone(),
                file_path: manifest.file_path.clone(),
                archive_sha256: manifest.sha256.clone(),
            },
        ],
        node_health_rows: vec![PoolUpstreamNodeHealthArchiveRow {
            archived_row_id: 11,
            occurred_at: "2026-10-26T01:00:00Z".to_owned(),
            proxy_binding_key_snapshot: "pool-a".to_owned(),
            is_success: 1,
            latency_ms: Some(12.5),
        }],
        node_health_hourly_rows: vec![PoolUpstreamNodeHealthHourlyArchiveRow {
            proxy_binding_key_snapshot: "pool-a".to_owned(),
            bucket_start_epoch: 1_792_972_800,
            success_count: 1,
            failure_count: 0,
        }],
        raw_reference_changes: Vec::new(),
        historical_activity: Vec::new(),
        retention_prepared_archive: None,
        retention_recovery_cursor: None,
        retention_raw_reconciliation: Vec::new(),
        manifest,
    };
    operation
        .validate(
            Path::new("/var/lib/codex/archive"),
            Path::new("/var/lib/codex/raw"),
        )
        .unwrap();

    let mut invalid = operation;
    invalid.replay_markers[0].archive_sha256 = digest("other");
    assert!(
        invalid
            .validate(
                Path::new("/var/lib/codex/archive"),
                Path::new("/var/lib/codex/raw"),
            )
            .is_err()
    );
}

#[test]
fn raw_conversion_preserves_legacy_aliases_without_filesystem_access() {
    let conversion = RawReferenceConversion::from_legacy_path(
        "raw/2026/10/request.bin",
        Path::new("/var/lib/codex/raw"),
    )
    .unwrap();
    assert_eq!(conversion.canonical_path, "2026/10/request.bin");
    assert!(
        conversion
            .aliases
            .contains(&"raw/2026/10/request.bin".to_owned())
    );
    assert!(
        conversion
            .aliases
            .contains(&"2026/10/request.bin".to_owned())
    );

    assert!(
        RawReferenceConversion::from_legacy_path(
            "../outside.bin",
            Path::new("/var/lib/codex/raw"),
        )
        .is_err()
    );
}

#[test]
fn activity_outbox_is_idempotent_and_rejects_newer_control_generation() {
    let item = HistoricalAccountActivityOutboxItem {
        archive_batch_id: 7,
        account_id: 11,
        last_activity_at: "2026-10-26T01:00:00Z".to_owned(),
        source_revision: 3,
        control_generation: 4,
    };
    assert_eq!(
        item.idempotency_key().unwrap(),
        item.idempotency_key().unwrap()
    );
    assert!(can_apply_historical_activity(&item, 4, None).unwrap());
    assert!(!can_apply_historical_activity(&item, 5, None).unwrap());
    assert!(!can_apply_historical_activity(&item, 4, Some("2026-10-26T02:00:00Z")).unwrap());
}

#[test]
fn activity_manifest_replacement_keeps_latest_observation_per_account() {
    let values = vec![
        HistoricalAccountActivityOutboxItem {
            archive_batch_id: 7,
            account_id: 11,
            last_activity_at: "2026-10-26T01:00:00Z".to_owned(),
            source_revision: 3,
            control_generation: 4,
        },
        HistoricalAccountActivityOutboxItem {
            archive_batch_id: 7,
            account_id: 11,
            last_activity_at: "2026-10-26T02:00:00Z".to_owned(),
            source_revision: 3,
            control_generation: 4,
        },
        HistoricalAccountActivityOutboxItem {
            archive_batch_id: 7,
            account_id: 12,
            last_activity_at: "2026-10-26T01:30:00Z".to_owned(),
            source_revision: 2,
            control_generation: 4,
        },
    ];
    let deduplicated = deduplicate_activity_manifest(7, &values).unwrap();
    assert_eq!(deduplicated.len(), 2);
    assert_eq!(
        deduplicated.get(&11).unwrap().last_activity_at,
        "2026-10-26T02:00:00Z"
    );
    assert!(deduplicate_activity_manifest(8, &values).is_err());
}

#[test]
fn prune_plan_requires_proof_rollup_and_zero_raw_references() {
    let manifest = manifest();
    let plan = ArchivePrunePlan {
        proof: proof(&manifest),
        rollups: vec![ArchiveRollupMarker {
            archive_batch_id: manifest.archive_batch_id,
            target: "hourly".to_owned(),
            bucket_start_epoch_ms: 100,
            source_identity_sha256: manifest.source_identity_sha256.clone(),
            materialized_rows: manifest.row_count,
            complete: true,
        }],
        raw_reference_checks: vec![RawReferenceCheck {
            raw_path: "2026/10/request.bin".to_owned(),
            active_reference_count: 0,
            file_identity_sha256: digest("raw"),
            checked_at_epoch_ms: 100,
        }],
        pending_activity_outbox: 0,
        file_identity_sha256: manifest.sha256.clone(),
        manifest,
    };
    plan.validate(
        Path::new("/var/lib/codex/archive"),
        Path::new("/var/lib/codex/raw"),
    )
    .unwrap();

    let mut unsafe_plan = plan.clone();
    unsafe_plan.raw_reference_checks[0].active_reference_count = 1;
    assert!(
        unsafe_plan
            .validate(
                Path::new("/var/lib/codex/archive"),
                Path::new("/var/lib/codex/raw"),
            )
            .is_err()
    );
}

#[test]
fn legacy_reader_bounded_projection_uses_nulls_for_missing_columns() {
    let schema = LegacyArchiveSchema::new(
        "codex_invocations",
        LegacyArchiveFormat::LegacyMonth,
        ["id", "occurred_at", "source"],
    )
    .unwrap();
    let sql = schema.bounded_select_sql(100).unwrap();
    assert!(sql.contains("\"source\" AS source"));
    assert!(sql.contains("NULL AS model"));
    assert!(sql.contains("LIMIT 100"));
    assert_eq!(schema.format_name(), "legacy_month");

    assert!(
        LegacyArchiveSchema::new(
            "codex_invocations",
            LegacyArchiveFormat::LegacyMonth,
            ["id"],
        )
        .is_err()
    );
}

#[test]
fn legacy_reader_maps_attempt_and_quota_archive_tables() {
    let attempts = LegacyArchiveSchema::new(
        "pool_upstream_request_attempts",
        LegacyArchiveFormat::SegmentV1,
        ["id", "occurred_at", "source"],
    )
    .unwrap();
    let attempts_sql = attempts.bounded_select_sql(10).unwrap();
    assert!(attempts_sql.contains("FROM \"pool_upstream_request_attempts\""));
    assert!(attempts_sql.contains("\"occurred_at\" AS occurred_at"));

    let quotas = LegacyArchiveSchema::new(
        "codex_quota_snapshots",
        LegacyArchiveFormat::LegacyMonth,
        ["id", "captured_at"],
    )
    .unwrap();
    let quota_sql = quotas.bounded_select_sql(10).unwrap();
    assert!(quota_sql.contains("\"captured_at\" AS occurred_at"));
    assert!(quota_sql.contains("FROM \"codex_quota_snapshots\""));
}

#[test]
fn legacy_month_manifest_may_omit_segment_keys() {
    let mut manifest = manifest();
    manifest.layout = "legacy_month".to_owned();
    manifest.day_key.clear();
    manifest
        .validate(Path::new("/var/lib/codex/archive"))
        .unwrap();
}

#[test]
fn retention_state_preserves_empty_cursor_and_requires_structured_sources() {
    let prepared = RetentionPreparedArchive {
        prepared_key: "prepared-7".to_owned(),
        dataset: "codex_invocations".to_owned(),
        month_key: "2026-10".to_owned(),
        day_key: None,
        part_key: None,
        file_path: "2026/10/part-0.sqlite.gz".to_owned(),
        source_ids_json: "[]".to_owned(),
        source_identity_sha256: digest("source"),
        publication_kind: Some("archive".to_owned()),
        state: "prepared".to_owned(),
        artifact_sha256: Some(digest("artifact")),
        artifact_bytes: Some(10),
        attempt_count: 0,
        next_retry_at: None,
        last_failure_stage: None,
        last_failure_fingerprint: None,
        staged_file_path: None,
        created_at_epoch_ms: 10,
        updated_at_epoch_ms: 10,
        quarantined_at: None,
    };
    prepared
        .validate(Path::new("/var/lib/codex/archive"))
        .unwrap();

    let mut invalid = prepared.clone();
    invalid.source_ids_json = "{}".to_owned();
    assert!(
        invalid
            .validate(Path::new("/var/lib/codex/archive"))
            .is_err()
    );

    RetentionRecoveryCursor {
        scope: "prepared_archives".to_owned(),
        cursor: String::new(),
        next_retry_at: None,
        consecutive_failure_count: 0,
        last_failure_fingerprint: None,
        defer_reason: None,
        last_progress_at: None,
        last_admission_stage: None,
        last_admission_cause: None,
        last_settled_pass_at: None,
        last_settled_pass_complete: None,
        last_settled_pass_inspected_entries: None,
        last_settled_pass_referenced_skipped: None,
        last_settled_pass_quarantined: None,
        last_settled_pass_removed: None,
        last_settled_pass_removed_bytes: None,
        last_nonzero_removal_at: None,
        last_nonzero_removal: None,
        last_nonzero_removal_bytes: None,
        updated_at_epoch_ms: 10,
    }
    .validate()
    .unwrap();
}

#[test]
fn absolute_paths_must_stay_under_the_configured_root() {
    let mut manifest = manifest();
    manifest.file_path = "/var/lib/codex/archive/2026/a.sqlite".to_owned();
    manifest
        .validate(Path::new("/var/lib/codex/archive"))
        .unwrap();
    manifest.file_path = "/var/lib/codex/other/a.sqlite".to_owned();
    assert!(
        manifest
            .validate(Path::new("/var/lib/codex/archive"))
            .is_err()
    );
}
