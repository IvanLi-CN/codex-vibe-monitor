use super::*;

#[test]
fn terminal_and_maintenance_deadlines_defer_expensive_repairs() {
    assert!(!long_term_projection_allows_expensive_repair(
        "terminal_deadline"
    ));
    assert!(long_term_projection_allows_expensive_repair(
        "repair_deadline"
    ));
    assert!(long_term_projection_allows_expensive_repair("daily_verify"));
    assert!(!long_term_projection_allows_expensive_repair(
        "maintenance_deadline"
    ));
}

#[test]
fn initial_materializer_only_runs_before_a_durable_long_term_baseline() {
    assert!(long_term_initial_materialization_needed(
        LONG_TERM_STATUS_PREPARING,
        None,
        false,
    ));
    assert!(long_term_initial_materialization_needed(
        LONG_TERM_STATUS_RUNNING,
        None,
        false,
    ));
    assert!(!long_term_initial_materialization_needed(
        LONG_TERM_STATUS_READY,
        None,
        false,
    ));
    assert!(!long_term_initial_materialization_needed(
        LONG_TERM_STATUS_PREPARING,
        None,
        true,
    ));
    assert!(long_term_initial_materialization_needed(
        LONG_TERM_STATUS_ERROR,
        None,
        true,
    ));
    assert!(long_term_initial_materialization_needed(
        LONG_TERM_STATUS_RUNNING,
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR),
        true,
    ));
}

#[test]
fn archive_scan_identity_requires_stable_bytes_and_matching_manifest() {
    assert!(long_term_archive_scan_identity_matches_manifest(
        "scanned-sha",
        Some("scanned-sha"),
        Some("scanned-sha"),
    ));
    assert!(!long_term_archive_scan_identity_matches_manifest(
        "scanned-sha",
        Some("rewritten-sha"),
        Some("scanned-sha"),
    ));
    assert!(!long_term_archive_scan_identity_matches_manifest(
        "scanned-sha",
        Some("scanned-sha"),
        Some("stale-manifest-sha"),
    ));
}

#[cfg(unix)]
#[test]
fn archive_file_identity_changes_when_a_prepared_replacement_is_renamed() {
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-replay-identity-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let replacement_path = archive_path.with_extension("replacement");
    fs::write(&archive_path, b"scanned archive bytes").expect("write scanned archive");
    let scanned_identity = long_term_archive_file_identity(archive_path.to_string_lossy().as_ref())
        .expect("read scanned archive identity");
    fs::write(&replacement_path, b"prepared replacement archive bytes")
        .expect("write prepared replacement archive");
    fs::rename(&replacement_path, &archive_path).expect("rename prepared replacement");
    let replacement_identity =
        long_term_archive_file_identity(archive_path.to_string_lossy().as_ref())
            .expect("read replacement archive identity");

    assert_ne!(scanned_identity, replacement_identity);
    let _ = fs::remove_file(&archive_path);
}

#[tokio::test]
async fn archive_source_identity_rejects_a_valid_file_with_a_stale_manifest() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-stale-archive-manifest-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&archive_path, b"valid replacement archive bytes")
        .expect("write replacement archive");
    let archive_path_text = archive_path.to_string_lossy().to_string();
    let archive_sha256 =
        crate::maintenance::sha256_hex_file(&archive_path).expect("hash replacement archive");
    sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('pool_upstream_request_attempts', '2026-08', ?1, 'stale-sha', 1, 'completed', datetime('now'))",
        )
        .bind(&archive_path_text)
        .execute(&pool)
        .await
        .expect("record stale manifest");

    let error = ensure_long_term_archive_source_identity(
        &pool,
        "pool_upstream_request_attempts",
        &archive_path_text,
        "stale-sha",
    )
    .await
    .expect_err("a valid replacement must not satisfy its stale manifest");
    assert!(error.to_string().contains("does not match"));

    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(&archive_sha256)
        .bind(&archive_path_text)
        .execute(&pool)
        .await
        .expect("update manifest to replacement identity");
    ensure_long_term_archive_source_identity(
        &pool,
        "pool_upstream_request_attempts",
        &archive_path_text,
        &archive_sha256,
    )
    .await
    .expect("matching manifest accepts the source");
    let _ = fs::remove_file(&archive_path);
}

#[test]
fn long_term_projection_repair_retries_deferred_and_persisted_dirty_work() {
    let now = Instant::now();

    assert!(long_term_projection_repair_due(
        false, true, false, None, now,
    ));
    assert!(long_term_projection_repair_due(
        false,
        false,
        false,
        Some(now - Duration::from_millis(1)),
        now,
    ));
    assert!(!long_term_projection_repair_due(
        false,
        false,
        false,
        Some(now + Duration::from_secs(1)),
        now,
    ));
}

#[test]
fn long_term_projection_repair_respects_a_future_deadline_with_pending_work() {
    let now = Instant::now();
    assert!(!long_term_projection_repair_due(
        true,
        true,
        true,
        Some(now + Duration::from_secs(1)),
        now,
    ));
}

#[tokio::test]
async fn daily_verify_pressure_rejection_stays_due_and_wakes_after_gate_release() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)")
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection state");
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("missing verification is due")
    );
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = gate
        .try_begin_background("test-daily-verify-pressure")
        .expect("hold background admission");
    let observed_generation = gate.eligibility_generation();
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let error = queue_long_term_projection_daily_verify_with_control(
        &pool,
        &Utc::now().with_timezone(&Shanghai).date_naive().to_string(),
        &control,
    )
    .await
    .expect_err("pressure must reject daily verification persistence");
    assert!(long_term_projection_write_is_deferred(&error));
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("pressure rejection keeps daily verification due")
    );
    let wait_for_release = wait_for_long_term_projection_pressure_retry(
        &gate,
        observed_generation,
        long_term_projection_pressure_retry_at(&gate),
    );
    drop(held);
    tokio::time::timeout(Duration::from_millis(100), wait_for_release)
        .await
        .expect("daily verification retry wakes as soon as the background slot releases");
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("the persisted verification obligation survives the pressure retry")
    );
    sqlx::query(
            "UPDATE long_term_projection_state SET last_daily_verify_at = datetime('now') WHERE consumer = ?1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("persist verification success");
    assert!(
        !long_term_projection_daily_verify_due(&pool)
            .await
            .expect("fresh verification is not due")
    );
}

#[tokio::test]
async fn existing_projection_cursor_retries_without_a_write_transaction_under_pressure() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 73)")
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection cursor");
    sqlx::query("PRAGMA query_only = ON")
        .execute(&pool)
        .await
        .expect("reject write transactions for the cursor read");

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = gate
        .try_begin_background("test-existing-cursor-pressure")
        .expect("hold P2 admission");
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let attempts = AtomicUsize::new(0);

    assert_eq!(
        run_long_term_projection_flush_with_retry_delays(
            &shutdown,
            || async {
                if attempts.fetch_add(1, Ordering::AcqRel) == 0 {
                    bail!("database is locked");
                }
                load_long_term_projection_cursor_with_control(&pool, &control).await
            },
            &[Duration::ZERO],
        )
        .await
        .expect("the retry reads an existing cursor without a write transaction"),
        73
    );
    assert_eq!(attempts.load(Ordering::Acquire), 2);
    drop(held);
}

#[tokio::test]
async fn existing_projection_cursor_read_cancels_while_waiting_for_sqlite() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 73)")
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection cursor");

    let held_connection = pool.acquire().await.expect("hold the only pool connection");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let mut cursor_read = Box::pin(load_long_term_projection_cursor_with_control(
        &pool, &control,
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut cursor_read)
            .await
            .is_err(),
        "the cursor read waits for SQLite while the only connection is held"
    );

    shutdown.cancel();
    let error = tokio::time::timeout(Duration::from_secs(1), &mut cursor_read)
        .await
        .expect("cancelled cursor read returns promptly")
        .expect_err("shutdown cancels a blocked cursor read");
    assert!(error.to_string().contains("cancelled"));
    drop(held_connection);
}

#[tokio::test]
async fn missing_projection_cursor_defers_to_a_waiting_p1_before_writing() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");

    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let active = coordinator
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::InteractiveProxy)
        .await;
    let p1 = tokio::spawn({
        let coordinator = coordinator.clone();
        async move {
            coordinator
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.snapshot().await.p1_waiter_count > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("P1 terminal waiter registers before cursor initialization");

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let error = load_long_term_projection_cursor_with_control(&pool, &control)
        .await
        .expect_err("missing cursor initialization must yield to queued P1 terminal work");
    assert!(long_term_projection_write_is_pressure_deferred(&error));
    assert_eq!(coordinator.snapshot().await.p2_waiter_count, 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_state WHERE consumer = ?1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .fetch_one(&pool)
        .await
        .expect("cursor state count"),
        0
    );

    drop(active);
    let p1_permit = tokio::time::timeout(Duration::from_secs(1), p1)
        .await
        .expect("P1 terminal is not starved by cursor initialization")
        .expect("P1 terminal task");
    assert_eq!(p1_permit.write_class(), "p1_terminal");
    drop(p1_permit);

    assert_eq!(
        load_long_term_projection_cursor_with_control(&pool, &control)
            .await
            .expect("P2 initializes the missing cursor after P1 completes"),
        0
    );
}

#[tokio::test]
async fn cursor_initialization_yields_when_p1_arrives_after_p2_admission() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    let held_connection = pool.acquire().await.expect("hold the only pool connection");
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let p2_permit = coordinator
        .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        .expect("P2 is admitted before P1 arrives");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let mut begin = Box::pin(control.begin_cursor_initialization(&pool, &coordinator));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut begin)
            .await
            .is_err(),
        "P2 initialization is waiting before it owns a SQLite transaction"
    );

    let p1 = tokio::spawn({
        let coordinator = coordinator.clone();
        async move {
            coordinator
                .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if coordinator.snapshot().await.p1_waiter_count > 0 {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("P1 terminal waiter registers after P2 admission");

    let error = tokio::time::timeout(Duration::from_secs(1), &mut begin)
        .await
        .expect("P2 initialization yields when P1 arrives")
        .expect_err("P2 must defer before it starts SQLite work");
    assert!(long_term_projection_write_is_pressure_deferred(&error));
    drop(p2_permit);
    drop(held_connection);

    let p1_permit = tokio::time::timeout(Duration::from_secs(1), p1)
        .await
        .expect("P1 terminal is released after P2 yields")
        .expect("P1 terminal task");
    assert_eq!(p1_permit.write_class(), "p1_terminal");
}

#[tokio::test]
async fn initial_materialization_control_rejects_progress_and_integrity_writes() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let mismatch = LongTermIntegrityMismatch {
        date: NaiveDate::from_ymd_opt(2026, 8, 20).expect("repair date"),
        expected: LongTermIntegrityTotals {
            calls: 2,
            ..LongTermIntegrityTotals::default()
        },
        observed: LongTermIntegrityTotals {
            calls: 1,
            ..LongTermIntegrityTotals::default()
        },
        reason: "controlled write test".to_string(),
    };
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = gate
        .try_begin_background("test-initial-materialization-pressure")
        .expect("hold background admission");
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);

    for error in [
        persist_long_term_refresh_progress(&pool, 256, 512, &control)
            .await
            .expect_err("pressure rejects progress write"),
        enqueue_long_term_integrity_mismatch(&pool, &mismatch, &control)
            .await
            .expect_err("pressure rejects repair queue write"),
        mark_long_term_integrity_audit(&pool, &control)
            .await
            .expect_err("pressure rejects audit marker write"),
    ] {
        assert!(long_term_projection_write_is_deferred(&error));
    }
    assert_eq!(
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT processed_rows, total_rows FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("unchanged progress"),
        (0, 0)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("empty repair queue"),
        0
    );
    assert!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_integrity_audit_at FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("missing audit marker")
        .is_none()
    );

    drop(held);
    shutdown.cancel();
    for error in [
        persist_long_term_refresh_progress(&pool, 256, 512, &control)
            .await
            .expect_err("shutdown rejects progress write"),
        enqueue_long_term_integrity_mismatch(&pool, &mismatch, &control)
            .await
            .expect_err("shutdown rejects repair queue write"),
        mark_long_term_integrity_audit(&pool, &control)
            .await
            .expect_err("shutdown rejects audit marker write"),
    ] {
        assert!(long_term_projection_write_is_deferred(&error));
    }
}

#[tokio::test]
async fn initial_materialization_control_waits_for_a_short_lived_background_slot() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = gate
        .try_begin_background("test-short-lived-background-slot")
        .expect("hold background admission");
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);

    let release = async move {
        sleep(Duration::from_millis(10)).await;
        drop(held);
    };
    let write = persist_long_term_refresh_progress(&pool, 256, 512, &control);
    let (_, result) = tokio::join!(release, write);
    result.expect("a short-lived background slot is admitted within the P2 window");
    assert_eq!(
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT processed_rows, total_rows FROM long_term_stats_state WHERE id = ?1",
        )
        .bind(LONG_TERM_STATE_ID)
        .fetch_one(&pool)
        .await
        .expect("persisted progress"),
        (256, 512)
    );
}

#[tokio::test]
async fn controlled_transaction_cancels_while_an_external_writer_holds_sqlite() {
    let (pool, _db_url, db_path) = long_term_file_backed_pool_with_busy_timeout(
        "cancel-locked-write",
        Duration::from_secs(30),
    )
    .await;
    let external_writer = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("hold the external SQLite writer lock");
    let shutdown = CancellationToken::new();
    let cancellation = shutdown.clone();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);

    let cancel = async move {
        sleep(Duration::from_millis(20)).await;
        cancellation.cancel();
    };
    let blocked_begin = tokio::time::timeout(Duration::from_millis(200), control.begin(&pool));
    let (_, result) = tokio::join!(cancel, blocked_begin);
    let error = result
        .expect("shutdown cancels the blocked transaction before SQLite's busy timeout")
        .expect_err("locked transaction admission is cancelled");
    assert!(long_term_projection_write_is_deferred(&error));
    let permit = gate
        .try_begin_background("verify-cancelled-transaction-released-permit")
        .expect("cancellation releases the P2 admission permit");
    drop(permit);
    external_writer
        .rollback()
        .await
        .expect("release external SQLite writer lock");
    cleanup_long_term_file_backed_pool(pool, db_path).await;
}

#[tokio::test]
async fn controlled_transaction_timeout_waits_for_pressure_cooldown_before_retry() {
    let (pool, _db_url, db_path) = long_term_file_backed_pool_with_busy_timeout(
        "cooldown-locked-write",
        Duration::from_secs(30),
    )
    .await;
    let external_writer = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .expect("hold the external SQLite writer lock");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_millis(50));
    let control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let observed_generation = gate.eligibility_generation();

    let error = tokio::time::timeout(Duration::from_millis(400), control.begin(&pool))
        .await
        .expect("bounded transaction admission")
        .expect_err("external writer keeps transaction admission blocked");
    assert!(long_term_projection_write_is_pressure_deferred(&error));
    let retry_at = long_term_projection_pressure_retry_at(&gate)
        .expect("transaction admission timeout starts a pressure cooldown");
    assert!(matches!(
        gate.background_deny_reason(),
        Some(crate::db_pressure::DbPressureDenyReason::PressureCooldown { .. })
    ));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(10),
            wait_for_long_term_projection_pressure_retry(
                &gate,
                observed_generation,
                Some(retry_at)
            ),
        )
        .await
        .is_err()
    );

    external_writer
        .rollback()
        .await
        .expect("release external SQLite writer lock");
    cleanup_long_term_file_backed_pool(pool, db_path).await;
}

#[tokio::test]
async fn daily_verify_stays_due_until_its_queued_bucket_is_rebuilt() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)")
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection state");
    let today = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    let control = LongTermProjectionWriteControl::unrestricted();
    queue_long_term_projection_daily_verify_with_control(&pool, &today, &control)
        .await
        .expect("persist daily verification pending marker and repair");
    queue_long_term_projection_daily_verify_with_control(&pool, &today, &control)
        .await
        .expect("pending daily verification is not queued twice");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT generation FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(&today)
        .fetch_one(&pool)
        .await
        .expect("single queued daily verification generation"),
        1
    );

    complete_long_term_projection_daily_verify_with_control(&pool, &today, true, &control)
        .await
        .expect("retention backlog keeps daily verification pending");
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("maintenance-pending verification remains due")
    );

    complete_long_term_projection_daily_verify_with_control(&pool, &today, false, &control)
        .await
        .expect("dirty daily verification remains pending after retention drains");
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("unrepaired verification remains due")
    );

    queue_long_term_projection_repairs(&pool, std::slice::from_ref(&today), "newer_generation")
        .await
        .expect("queue a newer daily repair");
    complete_long_term_projection_daily_verify_with_control(&pool, &today, false, &control)
        .await
        .expect("verified today bucket");
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("new dirty generation keeps verification due")
    );

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1")
        .bind(&today)
        .execute(&pool)
        .await
        .expect("clear completed dirty marker");
    complete_long_term_projection_daily_verify_with_control(&pool, &today, false, &control)
        .await
        .expect("verified clean today bucket");
    assert!(
        !long_term_projection_daily_verify_due(&pool)
            .await
            .expect("completed verification is fresh")
    );
}

#[tokio::test]
async fn daily_verify_pending_bucket_survives_calendar_rollover() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 0)")
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("seed projection state");
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let yesterday = today.pred_opt().expect("previous calendar date");
    let today_text = today.to_string();
    let yesterday_text = yesterday.to_string();
    sqlx::query(
        "UPDATE long_term_projection_state SET last_daily_verify_at = ?2 WHERE consumer = ?1",
    )
    .bind(LONG_TERM_PROJECTION_CONSUMER)
    .bind(format!("{yesterday_text} 15:59:59"))
    .execute(&pool)
    .await
    .expect("seed a prior Shanghai calendar verification");
    let control = LongTermProjectionWriteControl::unrestricted();
    assert_eq!(
        queue_long_term_projection_daily_verify_with_control(&pool, &yesterday_text, &control,)
            .await
            .expect("queue the original verification bucket"),
        yesterday_text
    );
    assert_eq!(
        queue_long_term_projection_daily_verify_with_control(&pool, &today_text, &control)
            .await
            .expect("recover the original pending verification bucket after midnight"),
        yesterday_text
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT daily_verify_bucket_date FROM long_term_projection_state WHERE consumer = ?1",
        )
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .fetch_one(&pool)
        .await
        .expect("persisted pending bucket"),
        yesterday_text
    );

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1")
        .bind(&yesterday_text)
        .execute(&pool)
        .await
        .expect("simulate the original date rebuild finishing after midnight");
    complete_long_term_projection_daily_verify_with_control(
        &pool,
        &yesterday_text,
        false,
        &control,
    )
    .await
    .expect("complete only the original pending bucket");
    assert!(
        long_term_projection_daily_verify_due(&pool)
            .await
            .expect("the current Shanghai calendar day remains due")
    );
    assert_eq!(
        queue_long_term_projection_daily_verify_with_control(&pool, &today_text, &control)
            .await
            .expect("queue the unverified current calendar day"),
        today_text
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(&today_text)
        .fetch_one(&pool)
        .await
        .expect("current day repair marker"),
        1
    );
}

#[tokio::test]
async fn bounded_ready_refresh_keeps_last_good_rows_after_a_cancelled_write() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let date_text = date.to_string();
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let mut seeded = pool.begin().await.expect("seed transaction");
    let mut daily = HashMap::new();
    for index in 0..=LONG_TERM_PROJECTION_WRITE_BATCH_ROWS {
        let series_key = format!("model:rebuilt-{index}");
        let bucket = LongTermBucket {
            bucket_start_epoch: start_epoch,
            dimension: "model".to_string(),
            series_key: series_key.clone(),
            display_name: "rebuilt".to_string(),
            reasoning_effort: String::new(),
            stats_date: Some(date_text.clone()),
            accumulator: LongTermAccumulator {
                calls: 1,
                ..LongTermAccumulator::default()
            },
        };
        let previous = LongTermBucket {
            series_key: format!("model:previous-{index}"),
            display_name: "previous".to_string(),
            ..bucket.clone()
        };
        insert_long_term_daily(&mut seeded, &previous)
            .await
            .expect("seed prior daily row");
        daily.insert((date_text.clone(), "model".to_string(), series_key), bucket);
    }
    seeded.commit().await.expect("commit prior daily rows");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("seed ready long-term state");

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let cancelling_control =
        LongTermProjectionWriteControl::cancelling_after(&shutdown, &gate, &committed_batches, 8);
    let recomputed_dates = HashSet::from([date]);
    apply_refresh_test_rollup(
        &pool,
        date,
        &daily,
        &recomputed_dates,
        &HashSet::new(),
        &cancelling_control,
    )
    .await
    .expect_err("cancellation after a 512-row replacement batch");
    assert!(shutdown.is_cancelled());
    assert_eq!(committed_batches.load(Ordering::Acquire), 8);
    assert_cancelled_refresh_preserves_previous_rows(&pool, &date_text).await;

    let recovery_control = LongTermProjectionWriteControl::unrestricted();
    apply_refresh_test_rollup(
        &pool,
        date,
        &daily,
        &recomputed_dates,
        &HashSet::new(),
        &recovery_control,
    )
    .await
    .expect("retry initial materialization");
    let rebuilt_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(&date_text)
    .fetch_one(&pool)
    .await
    .expect("rebuilt daily row count");
    assert_eq!(
        rebuilt_rows,
        (LONG_TERM_PROJECTION_WRITE_BATCH_ROWS + 1) as i64
    );
    let recovered_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("recovered status");
    assert_eq!(recovered_status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn cancelled_completed_repair_keeps_the_retry_queue_until_publication() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let date_text = date.to_string();
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let replacement = LongTermBucket {
        bucket_start_epoch: start_epoch,
        dimension: "model".to_string(),
        series_key: "model:replacement".to_string(),
        display_name: "replacement".to_string(),
        reasoning_effort: String::new(),
        stats_date: Some(date_text.clone()),
        accumulator: LongTermAccumulator {
            calls: 1,
            ..LongTermAccumulator::default()
        },
    };
    let daily = HashMap::from([(
        (
            date_text.clone(),
            "model".to_string(),
            replacement.series_key.clone(),
        ),
        replacement,
    )]);
    let recomputed_dates = HashSet::from([date]);
    let completed_integrity_repairs = HashSet::from([date]);
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 1, 0, 0, 0, 0, 0, 'repair pending')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed completed repair queue entry");

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let interrupted_control =
        LongTermProjectionWriteControl::stopping_after_completed_integrity_repairs(
            &shutdown, &gate,
        );
    let error = apply_refresh_test_rollup(
        &pool,
        date,
        &daily,
        &recomputed_dates,
        &completed_integrity_repairs,
        &interrupted_control,
    )
    .await
    .expect_err("cancellation before publication");
    assert!(long_term_projection_write_is_deferred(&error));
    assert!(shutdown.is_cancelled());
    assert_refresh_repair_remains_queued(&pool, &date_text).await;

    let recovery_control = LongTermProjectionWriteControl::unrestricted();
    apply_refresh_test_rollup(
        &pool,
        date,
        &daily,
        &recomputed_dates,
        &completed_integrity_repairs,
        &recovery_control,
    )
    .await
    .expect("retry completes the queued repair");
    assert_refresh_repair_published(&pool, &date_text).await;
}

#[tokio::test]
async fn refresh_publication_commits_state_before_cleanup_can_cancel() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let date_text = date.to_string();
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let previous = LongTermBucket {
        bucket_start_epoch: start_epoch,
        dimension: "model".to_string(),
        series_key: "model:previous".to_string(),
        display_name: "previous".to_string(),
        reasoning_effort: String::new(),
        stats_date: Some(date_text.clone()),
        accumulator: LongTermAccumulator {
            calls: 1,
            ..LongTermAccumulator::default()
        },
    };
    let replacement = LongTermBucket {
        series_key: "model:replacement".to_string(),
        display_name: "replacement".to_string(),
        ..previous.clone()
    };
    let mut seeded = pool.begin().await.expect("seed transaction");
    insert_long_term_daily(&mut seeded, &previous)
        .await
        .expect("seed prior daily row");
    seeded.commit().await.expect("commit prior daily row");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("seed ready long-term state");

    let daily = HashMap::from([(
        (
            date_text.clone(),
            "model".to_string(),
            replacement.series_key.clone(),
        ),
        replacement,
    )]);
    let recomputed_dates = HashSet::from([date]);
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let control =
        LongTermProjectionWriteControl::stopping_after_refresh_publication(&shutdown, &gate);
    let error = apply_refresh_test_rollup(
        &pool,
        date,
        &daily,
        &recomputed_dates,
        &HashSet::new(),
        &control,
    )
    .await
    .expect_err("test cancellation stops only publication cleanup");
    assert!(long_term_projection_write_is_deferred(&error));
    assert!(shutdown.is_cancelled());
    assert_refresh_published_before_cleanup(&pool, &date_text).await;

    let recovery_control = LongTermProjectionWriteControl::unrestricted();
    finish_long_term_projection_publication_cleanup(&pool, &recovery_control)
        .await
        .expect("recovery releases an already-published backup");
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE bucket_date = ?1 AND active_daily_backup_token IS NOT NULL",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("released cleanup pointer"),
            0
        );
}

#[tokio::test]
async fn refresh_archive_marker_requires_the_scanned_manifest_identity() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-refresh-marker-{}-{}.sqlite.gz",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&archive_path, b"scanned archive bytes").expect("write scanned archive");
    let archive_path = archive_path.to_string_lossy().to_string();
    let archive_sha256 = crate::maintenance::sha256_hex_file(std::path::Path::new(&archive_path))
        .expect("hash scanned archive");
    sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('codex_invocations', '2026-08', ?1, ?2, 1, 'completed', datetime('now'))",
        )
        .bind(&archive_path)
        .bind(&archive_sha256)
        .execute(&pool)
        .await
        .expect("record rewritten archive manifest");

    let control = LongTermProjectionWriteControl::unrestricted();
    let stale_marker = vec![(archive_path.clone(), "scanned-sha".to_string())];
    let no_failed_paths = HashSet::new();
    persist_test_archive_markers(&pool, &stale_marker, false, &no_failed_paths, &control)
        .await
        .expect_err("stale manifest identity rejects a replay marker");
    assert_archive_marker_count(&pool, &archive_path, 0).await;

    sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, 'prior-sha')",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&archive_path)
        .execute(&pool)
        .await
        .expect("seed stale replay marker");
    let changed_paths = HashSet::from([archive_path.clone()]);
    persist_test_archive_markers(&pool, &[], true, &changed_paths, &control)
        .await
        .expect("clear replay marker for a changed source");
    assert_archive_marker_count(&pool, &archive_path, 0).await;

    fs::write(&archive_path, b"replacement archive bytes")
        .expect("replace archive before marker persistence");
    let replacement_sha256 =
        crate::maintenance::sha256_hex_file(std::path::Path::new(&archive_path))
            .expect("hash replacement archive");
    let scanned_marker = vec![(archive_path.clone(), archive_sha256.clone())];
    persist_test_archive_markers(&pool, &scanned_marker, false, &no_failed_paths, &control)
        .await
        .expect_err("replacement before marker persistence must clear the marker");
    assert_archive_marker_count(&pool, &archive_path, 0).await;
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(&replacement_sha256)
        .bind(&archive_path)
        .execute(&pool)
        .await
        .expect("update replacement manifest");
    let current_marker = vec![(archive_path.clone(), replacement_sha256.clone())];
    persist_test_archive_markers(&pool, &current_marker, false, &no_failed_paths, &control)
        .await
        .expect("persist matching archive marker");
    let persisted_sha256 = sqlx::query_scalar::<_, String>(
            "SELECT archive_sha256 FROM hourly_rollup_archive_replay WHERE target = ?1 AND dataset = 'codex_invocations' AND file_path = ?2",
        )
        .bind(LONG_TERM_STATS_ARCHIVE_REPLAY_TARGET)
        .bind(&archive_path)
        .fetch_one(&pool)
        .await
        .expect("matching marker sha");
    assert_eq!(persisted_sha256, replacement_sha256);
    let _ = fs::remove_file(&archive_path);
}

#[tokio::test]
async fn published_backup_cleanup_recovers_after_pointer_release_cancellation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let date_text = date.to_string();
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let bucket = LongTermBucket {
        bucket_start_epoch: start_epoch,
        dimension: "model".to_string(),
        series_key: "model:previous".to_string(),
        display_name: "previous".to_string(),
        reasoning_effort: String::new(),
        stats_date: Some(date_text.clone()),
        accumulator: LongTermAccumulator {
            calls: 1,
            ..LongTermAccumulator::default()
        },
    };
    let mut seeded = pool.begin().await.expect("seed transaction");
    insert_long_term_daily(&mut seeded, &bucket)
        .await
        .expect("seed prior daily row");
    seeded.commit().await.expect("commit prior daily row");

    let rebuild_token = format!("long-term-date:{date_text}");
    let publication_token = "test-post-release-cleanup";
    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        &rebuild_token,
        false,
        &unrestricted,
    )
    .await
    .expect("create daily backup");
    stage_long_term_projection_date_publication(
        &pool,
        &date_text,
        &rebuild_token,
        publication_token,
        None,
        &unrestricted,
    )
    .await
    .expect("stage publication");
    sqlx::query(
            "INSERT INTO long_term_projection_date_publications (publication_token, published) VALUES (?1, 1)",
        )
        .bind(publication_token)
        .execute(&pool)
        .await
        .expect("publish replacement");

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let cancelling =
        LongTermProjectionWriteControl::stopping_after_backup_cleanup_marker(&shutdown, &gate);
    let error = release_long_term_projection_publication_member(
        &pool,
        &date_text,
        &rebuild_token,
        None,
        publication_token,
        &cancelling,
    )
    .await
    .expect_err("cancellation follows the durable cleanup marker");
    assert!(long_term_projection_write_is_deferred(&error));
    assert_cleanup_marker_after_cancel(&pool, &date_text, &rebuild_token).await;

    assert!(
        finish_long_term_projection_publication_cleanup(&pool, &unrestricted)
            .await
            .expect("remove one bounded backup cleanup batch")
    );
    assert!(
        finish_long_term_projection_publication_cleanup(&pool, &unrestricted)
            .await
            .expect("clear one bounded cleanup marker batch")
    );
    assert_cleanup_recovered(&pool, &date_text, &rebuild_token).await;
}

#[tokio::test]
async fn publication_cleanup_advances_one_cancelable_512_row_batch_per_maintenance_pass() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let rebuild_token = "bounded-publication-cleanup";
    for index in 0..1_025 {
        let bucket_date = format!("cleanup-date-{index:04}");
        sqlx::query(
                "INSERT INTO long_term_projection_bucket_state (bucket_date, publication_token) VALUES (?1, ?2)",
            )
            .bind(&bucket_date)
            .bind(format!("cleanup:{rebuild_token}"))
            .execute(&pool)
            .await
            .expect("seed cleanup marker");
        sqlx::query(
                "INSERT INTO long_term_projection_daily_backups (rebuild_token, stats_date, dimension, series_key, display_name) VALUES (?1, ?2, 'overall', 'overall', 'All')",
            )
            .bind(rebuild_token)
            .bind(bucket_date)
            .execute(&pool)
            .await
            .expect("seed backup cleanup row");
    }
    let control = LongTermProjectionWriteControl::unrestricted();
    assert!(
        finish_long_term_projection_publication_cleanup(&pool, &control)
            .await
            .expect("first bounded maintenance cleanup batch")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
        )
        .bind(rebuild_token)
        .fetch_one(&pool)
        .await
        .expect("one bounded backup batch remaining"),
        513
    );
    assert!(
        long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("durable cleanup backlog schedules the next maintenance pass")
    );

    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let cancelled = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let error = finish_long_term_projection_publication_cleanup(&pool, &cancelled)
        .await
        .expect_err("cancellation stops before the next cleanup write");
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
        )
        .bind(rebuild_token)
        .fetch_one(&pool)
        .await
        .expect("cancelled cleanup preserved the durable backlog"),
        513
    );

    while finish_long_term_projection_publication_cleanup(&pool, &control)
        .await
        .expect("resume one bounded cleanup batch at a time")
    {}
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_daily_backups WHERE rebuild_token = ?1",
        )
        .bind(rebuild_token)
        .fetch_one(&pool)
        .await
        .expect("drained cleanup backups"),
        0
    );
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_bucket_state WHERE publication_token IS NOT NULL",
            )
            .fetch_one(&pool)
            .await
            .expect("drained cleanup markers"),
            0
        );
    assert!(
        !long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("drained cleanup no longer schedules maintenance")
    );
}

#[tokio::test]
async fn interrupted_initial_refresh_retries_empty_source_without_publishing_partial_rollups() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let date = Utc::now().with_timezone(&Shanghai).date_naive().to_string();
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name) VALUES (?1, 'overall', 'overall', 'last good')",
        )
        .bind(&date)
        .execute(&pool)
        .await
        .expect("seed partial durable row");

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let control =
        LongTermProjectionWriteControl::cancelling_after(&shutdown, &gate, &committed_batches, 1);
    refresh_long_term_stats_once(&pool, 400, &control)
        .await
        .expect_err("cancelled initial refresh");
    assert!(shutdown.is_cancelled());

    let state: (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("interrupted initial state");
    assert_eq!(state.0, LONG_TERM_STATUS_RUNNING);
    assert_eq!(
        state.1.as_deref(),
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
    );
    assert!(long_term_initial_materialization_needed(
        &state.0,
        state.1.as_deref(),
        true,
    ));

    let recovery_control = LongTermProjectionWriteControl::unrestricted();
    refresh_long_term_stats_once(&pool, 400, &recovery_control)
        .await
        .expect("empty-source retry clears its interrupted initial prefix");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
            .fetch_one(&pool)
            .await
            .expect("empty-source retry daily rows"),
        0
    );
    let recovered_state: (String, Option<String>) =
        sqlx::query_as("SELECT status, last_error FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("empty-source recovery state");
    assert_eq!(recovered_state.0, LONG_TERM_STATUS_EMPTY);
    assert_eq!(recovered_state.1, None);
}
