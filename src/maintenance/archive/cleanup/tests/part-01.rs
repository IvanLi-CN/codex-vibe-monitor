use super::*;
use std::{thread, time::Duration};

#[test]
fn historical_rollup_materialization_remaining_budget_clamps_to_zero_when_elapsed() {
    let started_at = Instant::now();
    thread::sleep(Duration::from_millis(10));

    let remaining = historical_rollup_materialization_remaining_budget(
        started_at,
        Some(Duration::from_millis(1)),
    );

    assert_eq!(remaining, Some(Duration::ZERO));
}

#[test]
fn historical_rollup_materialization_remaining_budget_preserves_unbounded_mode() {
    assert_eq!(
        historical_rollup_materialization_remaining_budget(Instant::now(), None),
        None
    );
}

#[test]
fn historical_rollup_materialization_budget_exhausted_only_when_bounded_budget_is_zero() {
    assert!(historical_rollup_materialization_budget_exhausted(
        Instant::now(),
        Some(Duration::ZERO),
    ));
    assert!(!historical_rollup_materialization_budget_exhausted(
        Instant::now(),
        None,
    ));
    assert!(!historical_rollup_materialization_budget_exhausted(
        Instant::now(),
        Some(Duration::from_secs(1)),
    ));
}

#[test]
fn summary_snapshot_hash_state_resumes_at_persisted_offset() {
    let path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-summary-hash-{}.bin",
        nanoid::nanoid!()
    ));
    let source = b"first-second";
    fs::write(&path, source).expect("write synthetic archive source");
    let source_fingerprint =
        summary_archive_source_fingerprint(&path).expect("fingerprint synthetic archive source");
    let mut prefix_hasher = ResumableSha256::new();
    prefix_hasher.update(&source[..5]);
    let progress = SummaryArchiveSnapshotBackfillProgress {
        hash_algorithm: Some(SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM.to_string()),
        hash_state_version: SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION,
        hash_byte_offset: 5,
        hash_state: Some(prefix_hasher.serialize().to_vec()),
        source_fingerprint: Some(source_fingerprint.clone()),
        ..SummaryArchiveSnapshotBackfillProgress::default()
    };

    let actual = resumable_summary_archive_sha256(
        &path,
        progress,
        source_fingerprint,
        Instant::now(),
        Duration::from_secs(1),
    )
    .expect("resume synthetic archive hash");
    let expected = format!("{:x}", Sha256::digest(source));
    assert!(
        matches!(actual, SummaryArchiveHashAdvance::Complete { sha256, .. } if sha256 == expected)
    );
    fs::remove_file(&path).expect("remove synthetic archive source");
}

#[test]
fn summary_snapshot_hash_state_restarts_on_source_fingerprint_change() {
    let path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-summary-hash-fingerprint-{}.bin",
        nanoid::nanoid!()
    ));
    let source = b"replacement-source";
    fs::write(&path, source).expect("write synthetic archive source");
    let source_fingerprint =
        summary_archive_source_fingerprint(&path).expect("fingerprint synthetic archive source");
    let mut stale_hasher = ResumableSha256::new();
    stale_hasher.update(b"stale");
    let progress = SummaryArchiveSnapshotBackfillProgress {
        hash_algorithm: Some(SUMMARY_ARCHIVE_SNAPSHOT_HASH_ALGORITHM.to_string()),
        hash_state_version: SUMMARY_ARCHIVE_SNAPSHOT_HASH_STATE_VERSION,
        hash_byte_offset: 5,
        hash_state: Some(stale_hasher.serialize().to_vec()),
        source_fingerprint: Some("different-source".to_string()),
        ..SummaryArchiveSnapshotBackfillProgress::default()
    };

    let actual = resumable_summary_archive_sha256(
        &path,
        progress,
        source_fingerprint,
        Instant::now(),
        Duration::from_secs(1),
    )
    .expect("restart synthetic archive hash");
    let expected = format!("{:x}", Sha256::digest(source));
    assert!(
        matches!(actual, SummaryArchiveHashAdvance::Complete { sha256, .. } if sha256 == expected)
    );
    fs::remove_file(&path).expect("remove synthetic archive source");
}

#[tokio::test]
async fn startup_candidates_include_materialized_invocation_archive_missing_global_summary_proof() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                summary_source_kind,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                '/legacy/missing-global-summary-proof.sqlite.gz',
                'legacy-summary-proof',
                1,
                'completed',
                'unknown',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("seed materialized legacy invocation archive");
    for target in [
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay \
                 (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', \
                         '/legacy/missing-global-summary-proof.sqlite.gz', \
                         'legacy-summary-proof')",
        )
        .bind(target)
        .execute(&pool)
        .await
        .expect("seed retained summary proof");
    }
    crate::schema::ensure_schema(&pool)
        .await
        .expect("upgrade legacy archive schema");

    let candidates = load_historical_rollup_startup_candidates(&pool, 0)
        .await
        .expect("load startup candidates");

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].file_path,
        "/legacy/missing-global-summary-proof.sqlite.gz"
    );
}

#[tokio::test]
async fn summary_archive_snapshot_backfill_prioritizes_current_30d_horizon() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    let old_start = format_utc_iso(Utc::now() - ChronoDuration::days(90));
    let old_end = format_utc_iso(Utc::now() - ChronoDuration::days(89));
    let recent_start = format_utc_iso(Utc::now() - ChronoDuration::days(2));
    let recent_end = format_utc_iso(Utc::now() - ChronoDuration::days(1));
    for (id, path, sha, start, end) in [
        (
            1_i64,
            "/legacy/old-summary.sqlite.gz",
            "old-summary",
            old_start,
            old_end,
        ),
        (
            2_i64,
            "/legacy/recent-summary.sqlite.gz",
            "recent-summary",
            recent_start,
            recent_end,
        ),
    ] {
        sqlx::query(
                "INSERT INTO archive_batches \
                 (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind, \
                  coverage_start_at, coverage_end_at) \
                 VALUES (?1, 'codex_invocations', '2026-08', ?2, ?3, 1, 'completed', 'unknown', ?4, ?5)",
            )
            .bind(id)
            .bind(path)
            .bind(sha)
            .bind(start)
            .bind(end)
            .execute(&pool)
            .await
            .expect("seed Snapshot V2 candidate");
    }

    let candidates = load_summary_archive_snapshot_backfill_due_candidates(&pool, 1)
        .await
        .expect("load 30d-prioritized candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, 2);
}

#[tokio::test]
async fn summary_archive_snapshot_backfill_due_queue_bypasses_id_sweep_cursor() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    let recent_start = format_utc_iso(Utc::now() - ChronoDuration::days(2));
    let recent_end = format_utc_iso(Utc::now() - ChronoDuration::days(1));
    sqlx::query(
        "INSERT INTO archive_batches \
             (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind, \
              coverage_start_at, coverage_end_at) \
             VALUES (1, 'codex_invocations', '2026-08', \
                     '/legacy/due-summary-authority.sqlite.gz', 'due-summary', 1, \
                     'completed', 'unknown', ?1, ?2)",
    )
    .bind(recent_start)
    .bind(recent_end)
    .execute(&pool)
    .await
    .expect("seed due Summary authority");
    store_summary_archive_snapshot_backfill_checkpoint(&pool, 100, 100, false)
        .await
        .expect("seed stale ID sweep cursor");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run bounded Summary Snapshot backfill");

    assert_eq!(result.scanned_archive_batches, 1);
    assert_eq!(result.next_cursor_id, 100);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT failure_kind FROM summary_archive_snapshot_backfill_outcome \
                 WHERE archive_batch_id = 1 AND manifest_sha256 = 'due-summary'",
        )
        .fetch_one(&pool)
        .await
        .expect("due candidate outcome"),
        "missing_source"
    );
}

#[tokio::test]
async fn summary_archive_snapshot_backfill_persists_deadline_defer() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        "INSERT INTO archive_batches \
             (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind, \
              coverage_start_at, coverage_end_at) \
             VALUES (1, 'codex_invocations', '2026-08', \
                     '/legacy/deferred-summary-authority.sqlite.gz', 'deferred-summary', 1, \
                     'completed', 'unknown', '2026-08-01 00:00:00', '2026-08-01 01:00:00')",
    )
    .execute(&pool)
    .await
    .expect("seed deferred Summary authority");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::ZERO)
        .await
        .expect("run expired Summary Snapshot backfill");

    assert!(result.hit_budget);
    assert_eq!(
        sqlx::query_as::<_, (String, String)>(
            "SELECT disposition, failure_kind \
                 FROM summary_archive_snapshot_backfill_outcome \
                 WHERE archive_batch_id = 1 AND manifest_sha256 = 'deferred-summary'",
        )
        .fetch_one(&pool)
        .await
        .expect("deferred outcome"),
        ("deferred".to_string(), "budget".to_string())
    );
    assert_eq!(
        SummaryArchiveSnapshotBackfillDisposition::from_storage("deferred", "budget"),
        SummaryArchiveSnapshotBackfillDisposition::InProgress,
        "a committed budget boundary must resume on the next supervisor turn"
    );
}

#[tokio::test]
async fn completed_invocation_archive_requires_summary_publication_proof() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                summary_source_kind,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                '/staged/missing-summary-proof.sqlite.gz',
                'staged-summary-proof',
                1,
                'materializing',
                'authoritative',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("seed staged invocation archive");

    let error = sqlx::query(
        "UPDATE archive_batches SET status = 'completed' \
             WHERE dataset = 'codex_invocations' \
               AND file_path = '/staged/missing-summary-proof.sqlite.gz'",
    )
    .execute(&pool)
    .await
    .expect_err("publication must require every Summary proof");
    assert!(
        error
            .to_string()
            .contains("requires Summary publication proof")
    );
}

#[tokio::test]
async fn summary_archive_snapshot_backfill_localizes_missing_authority_and_advances_cursor() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            )
            VALUES (1, 'codex_invocations', '2026-08',
                    '/legacy/missing-summary-authority.sqlite.gz', 'missing-authority', 1,
                    'completed', 'unknown', '2026-08-01 00:00:00',
                    '2026-08-01 01:00:00')
            "#,
    )
    .execute(&pool)
    .await
    .expect("seed missing Summary archive authority");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run bounded Summary Snapshot backfill");
    assert_eq!(result.scanned_archive_batches, 1);
    assert_eq!(result.unavailable_archive_batches, 1);
    assert_eq!(result.materialized_archive_batches, 0);
    assert_eq!(result.next_cursor_id, 1);

    let outcome = sqlx::query_as::<_, (String, String)>(
        "SELECT disposition, failure_kind
             FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = 'missing-authority'",
    )
    .fetch_one(&pool)
    .await
    .expect("load missing-source outcome");
    assert_eq!(
        outcome,
        ("unavailable".to_string(), "missing_source".to_string())
    );
    let checkpoint = sqlx::query_as::<_, (i64, i64, i64)>(
        "SELECT next_archive_batch_id, manifest_high_watermark_id, completed
             FROM summary_archive_snapshot_backfill_checkpoint WHERE scope = ?1",
    )
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_BACKFILL_SCOPE)
    .fetch_one(&pool)
    .await
    .expect("load Summary Snapshot backfill checkpoint");
    assert_eq!(checkpoint, (1, 1, 1));
}

#[tokio::test]
async fn summary_archive_snapshot_backfill_reuses_verified_v2_without_raw_source() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    let manifest_sha256 = "verified-v2-authority";
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            )
            VALUES (1, 'codex_invocations', '2026-08',
                    '/legacy/already-snapshotted.sqlite.gz', ?1, 1,
                    'completed', 'unknown', '2026-08-01 00:00:00',
                    '2026-08-01 00:00:00')
            "#,
    )
    .bind(manifest_sha256)
    .execute(&pool)
    .await
    .expect("seed verified Summary archive authority");
    let payload = serde_json::to_vec(&vec![SummaryArchiveSnapshotV2Record {
        id: 1,
        invoke_id: "verified-v2-invocation".to_string(),
        occurred_at: "2026-08-01 00:00:00".to_string(),
        source: "proxy".to_string(),
        model: None,
        response_model: None,
        input_tokens: 0,
        output_tokens: 0,
        cache_input_tokens: 0,
        reasoning_tokens: 0,
        reasoning_effort: None,
        total_tokens: 0,
        cost: None,
        cost_input: None,
        cost_cache_write: None,
        cost_cache_read: None,
        cost_output: None,
        cost_reasoning: None,
        status: "success".to_string(),
        error_message: None,
        failure_kind: None,
        failure_class: None,
        is_actionable: false,
        upstream_account_id: None,
    }])
    .expect("encode verified V2 payload");
    let page = SummaryArchiveSnapshotPage {
        archive_batch_id: 1,
        manifest_sha256: manifest_sha256.to_string(),
        page_index: 0,
        coverage_start: "2026-08-01 00:00:00".to_string(),
        coverage_end: "2026-08-01 00:00:00".to_string(),
        row_count: 1,
        payload: zstd::stream::encode_all(payload.as_slice(), 1)
            .expect("compress verified V2 payload"),
    };
    let mut tx = pool.begin().await.expect("begin V2 proof transaction");
    store_summary_archive_snapshot_page_v2_tx(tx.as_mut(), &page)
        .await
        .expect("store verified V2 page");
    tx.commit().await.expect("commit verified V2 page");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("run V2 proof backfill");
    assert_eq!(result.materialized_archive_batches, 1);
    assert_eq!(result.unavailable_archive_batches, 0);
    let disposition: String = sqlx::query_scalar(
        "SELECT disposition FROM summary_archive_snapshot_backfill_outcome
             WHERE archive_batch_id = 1 AND manifest_sha256 = ?1",
    )
    .bind(manifest_sha256)
    .fetch_one(&pool)
    .await
    .expect("load verified V2 outcome");
    assert_eq!(disposition, "complete");
}

#[tokio::test]
async fn summary_archive_snapshot_backfill_retry_timestamp_is_sqlite_comparable() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                id, dataset, month_key, file_path, sha256, row_count, status,
                summary_source_kind, coverage_start_at, coverage_end_at
            )
            VALUES (1, 'codex_invocations', '2026-08',
                    '/legacy/retryable-summary-authority.sqlite.gz', 'retryable', 1,
                    'completed', 'unknown', datetime('now', '-1 day'),
                    datetime('now', '-1 day', '+1 hour'))
            "#,
    )
    .execute(&pool)
    .await
    .expect("seed retryable Summary archive authority");
    let candidate = load_summary_archive_snapshot_backfill_candidates(&pool, 0, 1)
        .await
        .expect("load candidate before outcome")[0]
        .clone();
    record_summary_archive_snapshot_backfill_outcome(
        &pool,
        &candidate,
        "unavailable",
        "missing_source",
        0,
        0,
    )
    .await
    .expect("persist retry outcome");
    let candidates = load_summary_archive_snapshot_backfill_candidates(&pool, 0, 1)
        .await
        .expect("load candidates after retry outcome");
    assert!(
        candidates.is_empty(),
        "future RFC3339 retry timestamp must be compared numerically by SQLite"
    );
    let due_candidates = load_summary_archive_snapshot_backfill_due_candidates(&pool, 1)
        .await
        .expect("load due candidates after retry outcome");
    assert!(
        due_candidates.is_empty(),
        "a recent retryable candidate must remain deferred until next_probe_at"
    );
}

#[tokio::test]
async fn direct_authoritative_invocation_archive_publication_requires_summary_proof() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");

    let error = sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                summary_source_kind,
                coverage_start_at,
                coverage_end_at,
                historical_rollups_materialized_at
            )
            VALUES (
                'codex_invocations',
                '2026-08',
                '/authoritative/missing-summary-proof.sqlite.gz',
                'missing-summary-proof',
                1,
                'completed',
                'authoritative',
                '2026-08-01 00:00:00',
                '2026-08-01 01:00:00',
                datetime('now')
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect_err("direct authoritative publication must require every Summary proof");
    assert!(
        error
            .to_string()
            .contains("requires Summary publication proof")
    );
}

#[tokio::test]
async fn legacy_segment_mirror_classification_requires_contiguous_live_id_coverage() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    for id in [41_i64, 42_i64, 43_i64] {
        sqlx::query(
            "INSERT INTO codex_invocations \
                 (id, invoke_id, occurred_at, source, status, payload, raw_response) \
                 VALUES (?1, ?2, '2026-08-01 00:00:00', 'xy', 'success', '{}', '')",
        )
        .bind(id)
        .bind(format!("legacy-mirror-{id}"))
        .execute(&pool)
        .await
        .expect("seed retained live invocation");
    }
    for (file_path, part_key) in [
        (
            "/legacy/proven-live-mirror.sqlite.gz",
            "part-0000000000000029-000000000000002a-0123456789abcdef",
        ),
        (
            "/legacy/ambiguous-source.sqlite.gz",
            "part-000000000000002b-000000000000002c-0123456789abcdef",
        ),
    ] {
        sqlx::query(
            r#"
                INSERT INTO archive_batches (
                    dataset,
                    month_key,
                    part_key,
                    file_path,
                    sha256,
                    row_count,
                    status,
                    layout,
                    coverage_start_at,
                    coverage_end_at,
                    historical_rollups_materialized_at
                )
                VALUES (
                    'codex_invocations',
                    '2026-08',
                    ?1,
                    ?2,
                    ?3,
                    1,
                    'completed',
                    'segment_v1',
                    '2026-08-01 00:00:00',
                    '2026-08-01 01:00:00',
                    datetime('now')
                )
                "#,
        )
        .bind(part_key)
        .bind(file_path)
        .bind(format!("legacy-{part_key}"))
        .execute(&pool)
        .await
        .expect("seed legacy segment manifest");
    }

    crate::schema::ensure_schema(&pool)
        .await
        .expect("classify legacy segment manifests");
    let states = sqlx::query_as::<_, (String, String)>(
        "SELECT file_path, summary_source_kind FROM archive_batches ORDER BY file_path",
    )
    .fetch_all(&pool)
    .await
    .expect("load classified archive source kinds");
    assert_eq!(
        states,
        vec![
            (
                "/legacy/ambiguous-source.sqlite.gz".to_string(),
                "unknown".to_string(),
            ),
            (
                "/legacy/proven-live-mirror.sqlite.gz".to_string(),
                "live_mirror".to_string(),
            ),
        ]
    );
}

#[tokio::test]
async fn finalization_keeps_a_reactivated_legacy_archive_file() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-reactivated-archive-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    fs::write(&archive_path, b"old archive content").expect("write old archive file");
    let old_sha256 = sha256_hex_file(&archive_path).expect("hash old archive file");
    let archive_path_string = archive_path.to_string_lossy().to_string();
    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', datetime('now'))
            "#,
    )
    .bind(&archive_path_string)
    .bind(&old_sha256)
    .execute(&pool)
    .await
    .expect("insert staged archive manifest");
    assert!(
        stage_archive_batch_deletion(
            &pool,
            1,
            "codex_quota_snapshots",
            &archive_path_string,
            &old_sha256,
            None,
        )
        .await
        .expect("stage old archive deletion")
    );

    fs::write(&archive_path, b"replacement archive content")
        .expect("replace archive file before manifest rewrite");
    let replacement_sha256 = sha256_hex_file(&archive_path).expect("hash replacement archive");
    sqlx::query("UPDATE archive_batches SET sha256 = ?1, cleanup_state = ?2 WHERE id = 1")
        .bind(&replacement_sha256)
        .bind(ARCHIVE_CLEANUP_STATE_ACTIVE)
        .execute(&pool)
        .await
        .expect("reactivate rewritten archive manifest");

    let finalized = finalize_archive_batch_file_deletion(
        &pool,
        1,
        "codex_quota_snapshots",
        &archive_path_string,
        &old_sha256,
    )
    .await
    .expect("ignore stale archive deletion finalizer");
    assert!(!finalized);
    assert_eq!(
        fs::read(&archive_path).expect("read replacement archive"),
        b"replacement archive content"
    );
    let manifest: (String, String) =
        sqlx::query_as("SELECT sha256, cleanup_state FROM archive_batches WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("load reactivated archive manifest");
    assert_eq!(manifest.0, replacement_sha256);
    assert_eq!(manifest.1, ARCHIVE_CLEANUP_STATE_ACTIVE);

    let _ = fs::remove_file(&archive_path);
}

#[tokio::test]
async fn failed_file_removal_does_not_advance_staged_source_boundary() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-pending-source-boundary-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    fs::write(&archive_path, b"pending archive content").expect("write pending archive file");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash pending archive file");
    let archive_path_string = archive_path.to_string_lossy().to_string();
    sqlx::query(
            r#"
            INSERT INTO archive_batches (
                id,
                dataset,
                month_key,
                file_path,
                sha256,
                row_count,
                status,
                cleanup_state,
                cleanup_source_safe_start_date,
                created_at
            )
            VALUES (1, 'codex_quota_snapshots', '2025-01', ?1, ?2, 1, 'completed', 'delete_pending', '2025-01-04', datetime('now'))
            "#,
        )
        .bind(&archive_path_string)
        .bind(&archive_sha256)
        .execute(&pool)
        .await
        .expect("insert pending archive manifest");

    let finalized = finalize_archive_batch_file_deletion_with_remove(
        &pool,
        1,
        "codex_quota_snapshots",
        &archive_path_string,
        &archive_sha256,
        |_| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "forced archive deletion failure",
            ))
        },
    )
    .await
    .expect("failed file removal should remain retryable");
    assert!(!finalized);
    assert!(archive_path.exists());
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load source boundary after failed removal");
    assert!(integrity_source_start.is_none());

    let finalized = finalize_archive_batch_file_deletion(
        &pool,
        1,
        "codex_quota_snapshots",
        &archive_path_string,
        &archive_sha256,
    )
    .await
    .expect("retry pending archive deletion");
    assert!(finalized);
    let integrity_source_start: Option<String> = sqlx::query_scalar(
        "SELECT integrity_source_start_date FROM long_term_stats_state WHERE id = 1",
    )
    .fetch_one(&pool)
    .await
    .expect("load source boundary after finalized removal");
    assert_eq!(integrity_source_start.as_deref(), Some("2025-01-04"));
}

#[tokio::test]
async fn summary_coverage_obligation_requeues_stale_complete_outcome_without_proof() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        "INSERT INTO archive_batches
             (id, dataset, month_key, file_path, sha256, row_count, status,
              summary_source_kind, coverage_start_at, coverage_end_at)
             VALUES (1, 'codex_invocations', '2026-09',
                     '/missing/stale-complete.sqlite.gz', 'stale-complete', 1,
                     'completed', 'unknown', datetime('now', '-1 day'), datetime('now'))",
    )
    .execute(&pool)
    .await
    .expect("seed stale-complete manifest");
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_outcome
             (archive_batch_id, manifest_sha256, disposition, failure_kind, next_probe_at)
             VALUES (1, 'stale-complete', 'complete', '', datetime('now', '+7 days'))",
    )
    .execute(&pool)
    .await
    .expect("seed stale complete outcome");

    let candidates = load_summary_archive_snapshot_backfill_due_candidates(&pool, 1)
        .await
        .expect("load obligation candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].sha256, "stale-complete");
}

#[tokio::test]
async fn terminal_verification_failure_skips_v2_promotion_until_manifest_sha_changes() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");

    sqlx::query(
        "INSERT INTO archive_batches
             (id, dataset, month_key, file_path, sha256, row_count, status,
              summary_source_kind, coverage_start_at, coverage_end_at)
             VALUES (1, 'codex_invocations', '2026-09',
                     '/missing/terminal-proof.sqlite.gz', 'terminal-proof', 1,
                     'completed', 'unknown', datetime('now', '-1 day'), datetime('now'))",
    )
    .execute(&pool)
    .await
    .expect("seed terminal manifest");
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_backfill_outcome
             (archive_batch_id, manifest_sha256, disposition, failure_kind, next_probe_at)
             VALUES (1, 'terminal-proof', 'unavailable', 'verification_failed', datetime('now'))",
    )
    .execute(&pool)
    .await
    .expect("seed terminal verification outcome");
    sqlx::query(
        "INSERT INTO summary_archive_snapshot
             (archive_batch_id, manifest_sha256, page_index, coverage_start, coverage_end,
              row_count, payload, payload_bytes, snapshot_sha256, format_version)
             VALUES (1, 'terminal-proof', 0, datetime('now', '-1 day'), datetime('now'),
                     1, ?1, 1, 'invalid-page-hash', ?2)",
    )
    .bind(vec![0_u8])
    .bind(SUMMARY_ARCHIVE_SNAPSHOT_V2)
    .execute(&pool)
    .await
    .expect("seed invalid V2 page");

    let candidates = load_summary_archive_snapshot_v2_promotion_candidates(&pool, 1)
        .await
        .expect("load terminal proof promotion candidates");
    assert!(candidates.is_empty());

    sqlx::query("UPDATE archive_batches SET sha256 = 'replacement-proof' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("replace manifest identity");
    sqlx::query(
        "UPDATE summary_archive_snapshot
             SET manifest_sha256 = 'replacement-proof', snapshot_sha256 = 'replacement-page-hash'
             WHERE archive_batch_id = 1 AND manifest_sha256 = 'terminal-proof'",
    )
    .execute(&pool)
    .await
    .expect("replace snapshot identity");
    let candidates = load_summary_archive_snapshot_v2_promotion_candidates(&pool, 1)
        .await
        .expect("load replacement manifest candidate");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].1, "replacement-proof");
}

#[tokio::test]
async fn summary_coverage_terminal_gap_is_not_retried_until_manifest_changes() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    sqlx::query(
        "INSERT INTO archive_batches
             (id, dataset, month_key, file_path, sha256, row_count, status,
              summary_source_kind, coverage_start_at, coverage_end_at)
             VALUES (1, 'codex_invocations', '2026-09',
                     '/missing/empty.sqlite.gz', 'terminal-empty', 0,
                     'completed', 'unknown', datetime('now', '-1 day'), datetime('now'))",
    )
    .execute(&pool)
    .await
    .expect("seed terminal manifest");

    let result = backfill_summary_archive_snapshots_v2_window(&pool, Duration::from_secs(1))
        .await
        .expect("record terminal gap");
    assert_eq!(result.unavailable_archive_batches, 1);
    let candidates = load_summary_archive_snapshot_backfill_due_candidates(&pool, 1)
        .await
        .expect("load post-gap candidates");
    assert!(candidates.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM summary_coverage_obligation
                 WHERE archive_batch_id = 1 AND manifest_sha256 = 'terminal-empty'",
        )
        .fetch_one(&pool)
        .await
        .expect("load terminal obligation state"),
        "terminal_gap"
    );
}
