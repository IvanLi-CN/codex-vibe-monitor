use super::*;

#[tokio::test]
async fn projection_interval_state_compacts_each_invocation_and_survives_reopen() {
    let (pool, db_url, db_path) = long_term_file_backed_pool("projection-interval-state").await;
    let start_ms = Shanghai
        .with_ymd_and_hms(2026, 7, 26, 10, 0, 0)
        .single()
        .expect("Shanghai start")
        .timestamp_millis();
    let segments = (1..=256)
        .map(|id| projection_interval_segment(id, start_ms, start_ms + 1_000))
        .collect::<Vec<_>>();
    let control = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(&pool, &segments, &control)
        .await
        .expect("canonical interval state");
    let state_rows =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("canonical state row count");
    assert_eq!(state_rows, 256);
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    apply_long_term_projection_incremental_with_runtime(
        &pool,
        &runtime,
        &HashMap::new(),
        &HashMap::new(),
        &[],
        321,
        0,
    )
    .await
    .expect("persist projection cursor with canonical state");
    pool.close().await;

    let reopened = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .expect("reopen durable projection state");
    let dates = HashSet::from(["2026-07-26".to_string()]);
    let index = load_long_term_projection_interval_index(&reopened, &dates)
        .await
        .expect("load canonical interval index");
    let key = projection_interval_key(
        "daily",
        "2026-07-26".to_string(),
        "model".to_string(),
        "model:gpt-5:high".to_string(),
    );
    assert_eq!(
        index.get(&key).expect("model daily union").duration_ms,
        1_000
    );
    assert_eq!(
        load_long_term_projection_cursor(&reopened)
            .await
            .expect("reopened projection cursor"),
        321
    );
    cleanup_long_term_file_backed_pool(reopened, db_path).await;
}

#[tokio::test]
async fn incremental_projection_reuses_interval_index_across_micro_batches() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, _) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let start_ms = start_epoch * 1_000;
    let control = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(
        &pool,
        &[projection_interval_segment(1, start_ms, start_ms + 1_000)],
        &control,
    )
    .await
    .expect("seed existing interval state");
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let first_segment = projection_interval_segment(2, start_ms + 1_000, start_ms + 2_000);
    let first_outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &HashMap::new(),
            daily: &HashMap::new(),
            segments: std::slice::from_ref(&first_segment),
        },
        2,
        1,
        &control,
    )
    .await
    .expect("first incremental micro-batch");
    assert_eq!(
        first_outcome,
        LongTermProjectionIncrementalOutcome::Published
    );

    // A same-date batch must reuse the loaded union. Clearing the backing rows makes an
    // accidental reload observable while leaving the in-memory cache unchanged.
    sqlx::query("DELETE FROM long_term_projection_interval_state")
        .execute(&pool)
        .await
        .expect("remove persisted rows after first cache load");
    let second_segment = projection_interval_segment(3, start_ms + 2_000, start_ms + 3_000);
    let second_outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &HashMap::new(),
            daily: &HashMap::new(),
            segments: std::slice::from_ref(&second_segment),
        },
        3,
        1,
        &control,
    )
    .await
    .expect("second incremental micro-batch");
    assert_eq!(
        second_outcome,
        LongTermProjectionIncrementalOutcome::Published
    );
    let runtime = runtime.lock().await;
    let daily_key = projection_interval_key(
        "daily",
        date.to_string(),
        "overall".to_string(),
        "overall".to_string(),
    );
    assert_eq!(
        runtime
            .interval_index
            .get(&daily_key)
            .expect("same-date cache entry")
            .duration_ms,
        3_000,
        "the second micro-batch must not discard the prior same-date union"
    );
}

#[tokio::test]
async fn canonical_interval_lookup_seeks_from_interval_end() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let start_ms = start_epoch * 1_000;
    let end_ms = end_epoch * 1_000;
    sqlx::query(
            "INSERT INTO long_term_projection_interval_state (invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms) VALUES (1, 'model:old', 'account:old', ?1, ?2), (2, 'model:current', 'account:current', ?3, ?4)",
        )
        .bind(start_ms - 86_400_000)
        .bind(start_ms - 1)
        .bind(start_ms + 1_000)
        .bind(start_ms + 2_000)
        .execute(&pool)
        .await
        .expect("seed canonical intervals");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(
            "EXPLAIN QUERY PLAN SELECT invocation_row_id FROM long_term_projection_interval_state WHERE interval_start_ms < ?1 AND interval_end_ms > ?2",
        )
        .bind(end_ms)
        .bind(start_ms)
        .fetch_all(&pool)
        .await
        .expect("canonical interval query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_long_term_projection_interval_state_end_start")
    }));

    let index = load_long_term_projection_interval_index(&pool, &HashSet::from([date.to_string()]))
        .await
        .expect("load canonical interval index");
    let key = projection_interval_key(
        "daily",
        date.to_string(),
        "model".to_string(),
        "model:current".to_string(),
    );
    assert_eq!(
        index.get(&key).expect("current interval union").duration_ms,
        1_000
    );
}

#[tokio::test]
async fn legacy_interval_migration_seeks_by_invocation_id() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
        "EXPLAIN QUERY PLAN {LEGACY_INTERVAL_PAGE_END_SQL}"
    ))
    .bind(512_i64)
    .bind(512_i64)
    .fetch_all(&pool)
    .await
    .expect("legacy migration query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("SEARCH")
            && detail.contains("idx_long_term_projection_intervals_invocation")
            && detail.contains("invocation_row_id>?")
    }));
}

#[tokio::test]
async fn legacy_interval_checkpoint_advances_past_invalid_page_without_losing_sources() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    ensure_long_term_projection_schema(&pool).await.unwrap();
    sqlx::query(
        "WITH RECURSIVE ids(id) AS (SELECT 1 UNION ALL SELECT id+1 FROM ids WHERE id<513)
         INSERT INTO long_term_projection_intervals(bucket_kind,bucket_date,bucket_key,dimension,series_key,invocation_row_id,interval_start_ms,interval_end_ms)
         SELECT 'daily','2026-07-26','2026-07-26','overall','overall',id,10,CASE WHEN id<=512 THEN 10 ELSE 20 END FROM ids",
    ).execute(&pool).await.unwrap();
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let commits = AtomicUsize::new(0);
    let first_owner = LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &commits, 1);
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &first_owner)
            .await
            .unwrap()
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT cursor_row_id FROM long_term_projection_state WHERE consumer='legacy_interval_migration_v1'")
        .fetch_one(&pool).await.unwrap(), 512);
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &first_owner)
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let resumed = LongTermProjectionWriteControl::unrestricted();
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &resumed)
            .await
            .unwrap()
    );
    let canonical: (i64, i64, i64) = sqlx::query_as("SELECT invocation_row_id,interval_start_ms,interval_end_ms FROM long_term_projection_interval_state")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(canonical, (513, 10, 20));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_intervals WHERE invocation_row_id<=512"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        512,
        "invalid authority is retained, never silently deleted"
    );
    pool.close().await;
}

#[tokio::test]
async fn legacy_interval_checkpoint_revisits_late_earlier_ids_after_bounded_cleanup() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    ensure_long_term_projection_schema(&pool).await.unwrap();
    let insert = "INSERT INTO long_term_projection_intervals(bucket_kind,bucket_date,bucket_key,dimension,series_key,invocation_row_id,interval_start_ms,interval_end_ms) VALUES('daily','2026-07-26','2026-07-26','model','model:preserved',?,10,20)";
    sqlx::query(insert)
        .bind(100_i64)
        .execute(&pool)
        .await
        .unwrap();
    let control = LongTermProjectionWriteControl::unrestricted();
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .unwrap()
    );
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .unwrap()
    );
    assert!(
        !migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .unwrap()
    );
    // Simulate a supported earlier writer or forward repair restoring an older source ID.
    sqlx::query(insert)
        .bind(50_i64)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .unwrap(),
        "bounded rewind is durable"
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT cursor_row_id FROM long_term_projection_state WHERE consumer='legacy_interval_migration_v1'")
        .fetch_one(&pool).await.unwrap(), 49);
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .unwrap()
    );
    let ids: Vec<i64> = sqlx::query_scalar("SELECT invocation_row_id FROM long_term_projection_interval_state ORDER BY invocation_row_id")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(ids, vec![50, 100]);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state WHERE model_series_key='model:preserved'")
        .fetch_one(&pool).await.unwrap(), 2);
    pool.close().await;
}

#[tokio::test]
async fn legacy_interval_fallback_honors_rebuild_suppressions() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = "2026-07-26";
    sqlx::query(
            "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('daily', ?1, ?1, 'overall', 'overall', 77, 1784992800000, 1784992801000)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("seed legacy-only interval");
    sqlx::query(
            "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (77, ?1)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("suppress removed legacy interval");

    let index = load_long_term_projection_interval_index(&pool, &HashSet::from([date.to_string()]))
        .await
        .expect("load legacy fallback");
    assert!(
        index.is_empty(),
        "suppressed legacy state must not resurrect"
    );
}

#[tokio::test]
async fn projection_interval_state_retention_prunes_canonical_state_and_metadata() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let retention_start = long_term_projection_hourly_retention_start_date(366);
    let old_date = retention_start.pred_opt().expect("old retention date");
    let old_start_ms = Shanghai
        .from_local_datetime(&old_date.and_hms_opt(0, 0, 0).expect("old start"))
        .single()
        .expect("old Shanghai start")
        .timestamp_millis();
    let retained_start_ms = Shanghai
        .from_local_datetime(
            &retention_start
                .and_hms_opt(0, 0, 0)
                .expect("retention start"),
        )
        .single()
        .expect("retained Shanghai start")
        .timestamp_millis();
    let control = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(
        &pool,
        &[
            projection_interval_segment(1, old_start_ms, old_start_ms + 1_000),
            projection_interval_segment(2, retained_start_ms, retained_start_ms + 1_000),
        ],
        &control,
    )
    .await
    .expect("seed canonical state");
    sqlx::query(
            "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (1, ?1)",
        )
        .bind(old_date.to_string())
        .execute(&pool)
        .await
        .expect("old suppression");
    sqlx::query(
            "INSERT INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES ('old-retention', 1)",
        )
        .execute(&pool)
        .await
        .expect("old rebuild membership");

    let mut pruned_intervals = 0;
    loop {
        let (_, pruned) =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &control)
                .await
                .expect("bounded retention pass");
        if pruned == 0 {
            break;
        }
        assert!(pruned <= LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64);
        pruned_intervals += pruned;
    }
    assert_eq!(pruned_intervals, 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 1",
        )
        .fetch_one(&pool)
        .await
        .expect("old state count"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 2",
        )
        .fetch_one(&pool)
        .await
        .expect("retained state count"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_suppressions",
        )
        .fetch_one(&pool)
        .await
        .expect("suppression cleanup"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_rebuild_members",)
            .fetch_one(&pool)
            .await
            .expect("member cleanup"),
        0
    );
}

#[tokio::test]
async fn projection_interval_retention_batches_defer_cancel_and_resume() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let retention_start = long_term_projection_hourly_retention_start_date(366);
    let expired_date = retention_start.pred_opt().expect("expired retention date");
    let expired_start_ms = Shanghai
        .from_local_datetime(&expired_date.and_hms_opt(0, 0, 0).expect("expired start"))
        .single()
        .expect("expired Shanghai start")
        .timestamp_millis();
    let retained_start_ms = Shanghai
        .from_local_datetime(
            &retention_start
                .and_hms_opt(0, 0, 0)
                .expect("retained start"),
        )
        .single()
        .expect("retained Shanghai start")
        .timestamp_millis();
    let mut segments = (1..=1_025)
        .map(|id| projection_interval_segment(id, expired_start_ms, expired_start_ms + 1_000))
        .collect::<Vec<_>>();
    segments.push(projection_interval_segment(
        2_000,
        retained_start_ms,
        retained_start_ms + 1_000,
    ));
    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(&pool, &segments, &unrestricted)
        .await
        .expect("seed canonical interval state");
    for invocation_row_id in 1..=1_025 {
        sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES ('hourly', ?1, ?2, 'overall', 'overall', ?3, ?4, ?5)",
            )
            .bind(expired_date.to_string())
            .bind(format!("retention-{invocation_row_id}"))
            .bind(invocation_row_id)
            .bind(expired_start_ms)
            .bind(expired_start_ms + 1_000)
            .execute(&pool)
            .await
            .expect("seed expanded legacy interval");
        sqlx::query(
                "INSERT INTO long_term_projection_interval_suppressions (invocation_row_id, bucket_date) VALUES (?1, ?2)",
            )
            .bind(invocation_row_id)
            .bind(expired_date.to_string())
            .execute(&pool)
            .await
            .expect("seed expired suppression");
        sqlx::query(
                "INSERT INTO long_term_projection_rebuild_members (rebuild_token, invocation_row_id) VALUES (?1, ?2)",
            )
            .bind(format!("retention-{invocation_row_id}"))
            .bind(invocation_row_id)
            .execute(&pool)
            .await
            .expect("seed expired rebuild member");
    }

    let pressure_shutdown = CancellationToken::new();
    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = pressure_gate
        .try_begin_background("retention-test-holder")
        .expect("hold background admission");
    let pressure_control =
        LongTermProjectionWriteControl::background(&pressure_shutdown, &pressure_gate);
    let deferred =
        prune_long_term_projection_hourly_retention_with_control(&pool, 366, &pressure_control)
            .await
            .expect_err("retention must defer before opening a write transaction under pressure");
    assert!(long_term_projection_write_is_deferred(&deferred));
    drop(held);
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id <= 1025",
            )
            .fetch_one(&pool)
            .await
            .expect("canonical rows remain after pressure deferral"),
            1_025
        );

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let interrupted =
        LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
    let (_, pruned_intervals) =
        prune_long_term_projection_hourly_retention_with_control(&pool, 366, &interrupted)
            .await
            .expect("one bounded retention batch commits before cancellation");
    assert_eq!(
        pruned_intervals,
        LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64
    );
    assert_eq!(committed_batches.load(Ordering::Acquire), 1);
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id <= 1025",
            )
            .fetch_one(&pool)
            .await
                .expect("canonical rows remain before their retention turn"),
            1_025
        );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("one legacy retention batch leaves durable work for the next pass"),
        513
    );
    assert!(
        long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("durable maintenance backlog remains scheduled after one batch")
    );
    let error = prune_long_term_projection_hourly_retention_with_control(&pool, 366, &interrupted)
        .await
        .expect_err("cancellation stops the next maintenance pass before its write");
    assert!(error.to_string().contains("cancelled"));

    let mut resumed_pruned_intervals = 0;
    loop {
        let (_, pruned) =
            prune_long_term_projection_hourly_retention_with_control(&pool, 366, &unrestricted)
                .await
                .expect("retention resumes after cancellation");
        if pruned == 0 {
            break;
        }
        assert!(pruned <= LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as u64);
        resumed_pruned_intervals += pruned;
    }
    assert_eq!(resumed_pruned_intervals, 3_588);
    assert!(
        !long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("drained maintenance no longer schedules a tick")
    );
    for table in [
        "long_term_projection_interval_state",
        "long_term_projection_interval_suppressions",
        "long_term_projection_rebuild_members",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!(
                "SELECT COUNT(*) FROM {table} WHERE invocation_row_id <= 1025"
            ))
            .fetch_one(&pool)
            .await
            .expect("expired retention rows removed after resume"),
            0,
            "{table} must resume bounded cleanup"
        );
    }
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 2000",
            )
            .fetch_one(&pool)
            .await
            .expect("retained canonical row"),
            1
        );
}

#[tokio::test]
async fn projection_interval_state_batches_stop_for_cancel_and_pressure() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    let start_ms = Shanghai
        .with_ymd_and_hms(2026, 7, 26, 10, 0, 0)
        .single()
        .expect("Shanghai start")
        .timestamp_millis();
    let segments = (1..=1_025)
        .map(|id| projection_interval_segment(id, start_ms, start_ms + 1_000))
        .collect::<Vec<_>>();
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let cancel_control =
        LongTermProjectionWriteControl::cancelling_after(&shutdown, &gate, &committed_batches, 1);
    let cancelled =
        upsert_long_term_projection_interval_segments(&pool, &segments, &cancel_control)
            .await
            .expect_err("shutdown cancels the next internal write batch");
    assert!(cancelled.to_string().contains("cancelled"));
    assert!(shutdown.is_cancelled());
    assert_eq!(committed_batches.load(Ordering::Acquire), 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state",)
            .fetch_one(&pool)
            .await
            .expect("committed canonical rows"),
        LONG_TERM_PROJECTION_WRITE_BATCH_ROWS as i64
    );

    let pressure_shutdown = CancellationToken::new();
    let pressure_gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = pressure_gate
        .try_begin_background("test-holder")
        .expect("hold background admission");
    let pressure_control =
        LongTermProjectionWriteControl::background(&pressure_shutdown, &pressure_gate);
    let deferred = upsert_long_term_projection_interval_segments(
        &pool,
        &[projection_interval_segment(
            2_000,
            start_ms,
            start_ms + 1_000,
        )],
        &pressure_control,
    )
    .await
    .expect_err("pressure must defer before opening a write transaction");
    assert!(long_term_projection_write_is_deferred(&deferred));
    drop(held);
}

#[tokio::test]
async fn incremental_projection_defers_before_interval_and_rollup_publication() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let event = build_long_term_projection_event(&LongTermInvocationRow {
        id: 1,
        invoke_id: Some("atomic-pressure".to_string()),
        occurred_at: "2026-07-26 10:00:00".to_string(),
        status: Some("success".to_string()),
        model: Some("gpt-5".to_string()),
        request_model: None,
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: Some(100),
        output_tokens: None,
        cost: None,
        t_total_ms: Some(1_000.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        error_message: None,
    });
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let held = gate
        .try_begin_background("test-holder")
        .expect("hold background admission");
    let pressure_control = LongTermProjectionWriteControl::background(&shutdown, &gate);
    let deferred = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &event.hourly,
            daily: &event.daily,
            segments: &event.segments,
        },
        event.row_id,
        1,
        &pressure_control,
    )
    .await
    .expect_err("pressure rejects the atomic incremental publication");
    assert!(long_term_projection_write_is_deferred(&deferred));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("no canonical interval before publication"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
            .fetch_one(&pool)
            .await
            .expect("no rollup before publication"),
        0
    );
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE((SELECT cursor_row_id FROM long_term_projection_state WHERE consumer = ?1), 0)",
            )
            .bind(LONG_TERM_PROJECTION_CONSUMER)
            .fetch_one(&pool)
            .await
            .expect("projection cursor"),
            0
        );

    drop(held);
    let control = LongTermProjectionWriteControl::unrestricted();
    apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &event.hourly,
            daily: &event.daily,
            segments: &event.segments,
        },
        event.row_id,
        1,
        &control,
    )
    .await
    .expect("retry publishes the entire event once");
    let daily = sqlx::query_as::<_, (i64, i64)>(
            "SELECT calls, token_total FROM long_term_usage_daily WHERE stats_date = '2026-07-26' AND dimension = 'overall' AND series_key = 'overall'",
        )
        .fetch_one(&pool)
        .await
        .expect("daily projection");
    assert_eq!(daily, (1, 100));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("canonical interval after retry"),
        1
    );
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("advanced projection cursor"),
        event.row_id
    );
}

#[tokio::test]
async fn incremental_projection_rebuilds_when_a_dirty_marker_arrives_after_ready_read() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let event = build_long_term_projection_event(&LongTermInvocationRow {
        id: 1,
        invoke_id: Some("dirty-race".to_string()),
        occurred_at: "2026-07-26 10:00:00".to_string(),
        status: Some("success".to_string()),
        model: Some("gpt-5".to_string()),
        request_model: None,
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: Some(100),
        output_tokens: None,
        cost: None,
        t_total_ms: Some(1_000.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        error_message: None,
    });
    let date = "2026-07-26";
    sqlx::query(
            "INSERT INTO long_term_projection_bucket_state (bucket_date, interval_baseline_ready) VALUES (?1, 1)",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("seed ready bucket");
    let ready_dates = load_long_term_projection_ready_dates(&pool, &event.bucket_dates)
        .await
        .expect("ready snapshot");
    assert!(event.bucket_dates.is_subset(&ready_dates));

    // A correction can land after the ready read and before the incremental transaction.
    sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'test-race')",
        )
        .bind(date)
        .execute(&pool)
        .await
        .expect("queue correction after ready read");
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));
    let control = LongTermProjectionWriteControl::unrestricted();
    let outcome = apply_long_term_projection_incremental_with_runtime_and_control(
        &pool,
        &runtime,
        LongTermProjectionIncrementalBatch {
            hourly: &event.hourly,
            daily: &event.daily,
            segments: &event.segments,
        },
        event.row_id,
        1,
        &control,
    )
    .await
    .expect("dirty revalidation");
    assert_eq!(
        outcome,
        LongTermProjectionIncrementalOutcome::RebuildRequired
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("no canonical interval publication"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_usage_daily")
            .fetch_one(&pool)
            .await
            .expect("no mixed daily publication"),
        0
    );
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("unadvanced cursor"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(date)
        .fetch_one(&pool)
        .await
        .expect("durable dirty marker"),
        1
    );
}

#[tokio::test]
async fn legacy_projection_intervals_migrate_without_losing_the_canonical_interval() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    for (bucket_kind, bucket_key, dimension, series_key) in [
        ("daily", "2026-07-26", "overall", "overall"),
        ("daily", "2026-07-26", "model", "model:gpt-5:high"),
        ("daily", "2026-07-26", "upstream", "account:42"),
        ("hourly", "1785031200", "overall", "overall"),
        ("hourly", "1785031200", "model", "model:gpt-5:high"),
        ("hourly", "1785031200", "upstream", "account:42"),
    ] {
        sqlx::query(
                "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, '2026-07-26', ?2, ?3, ?4, 7, 10, 20)",
            )
            .bind(bucket_kind)
            .bind(bucket_key)
            .bind(dimension)
            .bind(series_key)
            .execute(&pool)
            .await
            .expect("legacy expanded interval");
    }
    let control = LongTermProjectionWriteControl::unrestricted();
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .expect("canonical legacy interval migration")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_interval_state WHERE invocation_row_id = 7",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical migration state"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("legacy expansion remains for a later bounded cleanup"),
        6
    );
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &control)
            .await
            .expect("bounded legacy cleanup")
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("legacy cleanup complete"),
        0
    );
}

#[tokio::test]
async fn legacy_projection_interval_migration_compresses_expansion_in_one_cancellable_batch() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    for invocation_row_id in 1..=256 {
        for bucket_kind in ["daily", "hourly"] {
            for (dimension, series_key) in [
                ("overall", "overall"),
                ("model", "model:gpt-5:high"),
                ("upstream", "account:42"),
            ] {
                sqlx::query(
                        "INSERT INTO long_term_projection_intervals (bucket_kind, bucket_date, bucket_key, dimension, series_key, invocation_row_id, interval_start_ms, interval_end_ms) VALUES (?1, '2026-07-26', ?2, ?3, ?4, ?5, 10, 20)",
                    )
                    .bind(bucket_kind)
                    .bind(format!("{bucket_kind}-{invocation_row_id}"))
                    .bind(dimension)
                    .bind(series_key)
                    .bind(invocation_row_id)
                    .execute(&pool)
                    .await
                    .expect("legacy expanded interval");
            }
        }
    }

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let interrupted =
        LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
    assert!(
        migrate_long_term_projection_legacy_interval_state(&pool, &interrupted)
            .await
            .expect("one canonical compression batch")
    );
    assert_eq!(committed_batches.load(Ordering::Acquire), 1);
    let interrupted_state = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE invocation_row_id = 9",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical interval after interrupted cleanup");
    assert_eq!(interrupted_state.model_series_key, "model:gpt-5:high");
    assert_eq!(interrupted_state.upstream_series_key, "account:42");
    assert_eq!(interrupted_state.interval_start_ms, 10);
    assert_eq!(interrupted_state.interval_end_ms, 20);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("all expanded rows remain before cleanup"),
        1_536
    );
    assert!(
        long_term_projection_maintenance_needed(&pool, 366)
            .await
            .expect("legacy expansion keeps the next maintenance pass scheduled")
    );
    let error = migrate_long_term_projection_legacy_interval_state(&pool, &interrupted)
        .await
        .expect_err("cancellation stops the next legacy cleanup batch");
    assert!(error.to_string().contains("cancelled"));

    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    for expected_remaining in [1_024_i64, 512, 0] {
        assert!(
            migrate_long_term_projection_legacy_interval_state(&pool, &unrestricted)
                .await
                .expect("resume one bounded legacy cleanup batch")
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
                .fetch_one(&pool)
                .await
                .expect("remaining expanded legacy rows"),
            expected_remaining
        );
    }
    assert!(
        !migrate_long_term_projection_legacy_interval_state(&pool, &unrestricted)
            .await
            .expect("completed migration has no further work")
    );
    let resumed_state = sqlx::query_as::<_, LongTermProjectionIntervalStateRow>(
            "SELECT invocation_row_id, model_series_key, upstream_series_key, interval_start_ms, interval_end_ms FROM long_term_projection_interval_state WHERE invocation_row_id = 9",
        )
        .fetch_one(&pool)
        .await
        .expect("canonical interval after resumed cleanup");
    assert_eq!(resumed_state.model_series_key, "model:gpt-5:high");
    assert_eq!(resumed_state.upstream_series_key, "account:42");
    assert_eq!(resumed_state.interval_start_ms, 10);
    assert_eq!(resumed_state.interval_end_ms, 20);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_intervals")
            .fetch_one(&pool)
            .await
            .expect("completed legacy cleanup"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_interval_state")
            .fetch_one(&pool)
            .await
            .expect("one durable interval state per invocation"),
        256
    );
}

#[tokio::test]
async fn interrupted_date_rebuild_keeps_cross_day_interval_for_neighboring_date() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let first_date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("first date");
    let second_date = first_date.succ_opt().expect("second date");
    let (first_start, first_end) = long_term_day_epoch_bounds(first_date).expect("first bounds");
    let (_, second_end) = long_term_day_epoch_bounds(second_date).expect("second bounds");
    let segment =
        projection_interval_segment(11, first_end * 1_000 - 1_000, second_end * 1_000 - 1_000);
    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    upsert_long_term_projection_interval_segments(&pool, &[segment], &unrestricted)
        .await
        .expect("seed cross-day canonical interval");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let cancelling_control =
        LongTermProjectionWriteControl::stopping_after(&shutdown, &gate, &committed_batches, 1);
    let rebuild = LongTermProjectionDateRebuild {
        bucket_date: first_date.to_string(),
        start_epoch: first_start,
        end_epoch: first_end,
        hourly: HashMap::new(),
        daily: HashMap::new(),
        interval_segments: Vec::new(),
        source_row_count: 0,
    };
    commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        &[rebuild],
        None,
        &[],
        false,
        &cancelling_control,
    )
    .await
    .expect_err("rebuild cancellation after the staging batch");
    let neighboring_index =
        load_long_term_projection_interval_index(&pool, &HashSet::from([second_date.to_string()]))
            .await
            .expect("neighboring durable interval index");
    assert!(
        neighboring_index
            .values()
            .any(|union| union.duration_ms > 0)
    );
}

#[tokio::test]
async fn daily_backup_claim_keeps_competing_rebuild_from_publishing_partial_rows() {
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
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls) VALUES (?1, 'model', 'model:last-good', 'last good', 41)",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed last-good daily row");
    sqlx::query(
            "INSERT INTO long_term_projection_daily_backup_claims (bucket_date, rebuild_token) VALUES (?1, 'owner-one')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("reserve first owner");
    let control = LongTermProjectionWriteControl::unrestricted();

    let error = ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        "owner-two",
        true,
        &control,
    )
    .await
    .expect_err("competing owner cannot replace an uncommitted snapshot");
    assert!(error.to_string().contains("owner-one"));
    let public_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
        .await
        .expect("public live rows while snapshot is reserved");
    assert_eq!(public_rows.len(), 1);
    assert_eq!(public_rows[0].series_key, "model:last-good");
    assert_eq!(public_rows[0].calls, 41);

    ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        "owner-one",
        true,
        &control,
    )
    .await
    .expect("first owner completes a snapshot");
    let active = sqlx::query_scalar::<_, Option<String>>(
            "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("active backup owner");
    assert_eq!(active.as_deref(), Some("owner-one"));
    let error = ensure_long_term_projection_daily_backup_for_date(
        &pool,
        &date_text,
        "owner-two",
        true,
        &control,
    )
    .await
    .expect_err("competing owner cannot replace a published snapshot");
    assert!(error.to_string().contains("owner-one"));

    release_long_term_projection_daily_backups(
        &pool,
        &[(date_text.clone(), "owner-one".to_string())],
        &control,
    )
    .await
    .expect("release completed owner");
    let claims = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_projection_daily_backup_claims WHERE bucket_date = ?1",
    )
    .bind(&date_text)
    .fetch_one(&pool)
    .await
    .expect("released claim");
    assert_eq!(claims, 0);
}

#[tokio::test]
async fn initial_marker_blocks_a_stale_baseline_from_publishing_ready() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1, last_error = ?2 WHERE id = ?3")
        .bind(LONG_TERM_STATUS_RUNNING)
        .bind(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("persist incomplete initial marker");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let rebuild = LongTermProjectionDateRebuild {
        bucket_date: date.to_string(),
        start_epoch,
        end_epoch,
        hourly: HashMap::new(),
        daily: HashMap::new(),
        interval_segments: Vec::new(),
        source_row_count: 0,
    };
    let control = LongTermProjectionWriteControl::unrestricted();

    let error = commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        &[rebuild],
        Some(321),
        &[],
        true,
        &control,
    )
    .await
    .expect_err("stale P2 baseline must not publish over the initial marker");
    assert!(
        error
            .to_string()
            .contains("incomplete initial materialization")
    );
    let state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("initial state remains retryable");
    assert_eq!(state.0, LONG_TERM_STATUS_RUNNING);
    assert_eq!(
        state.1.as_deref(),
        Some(LONG_TERM_INITIAL_MATERIALIZATION_PENDING_ERROR)
    );
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("cursor remains before rejected publication"),
        0
    );
}

#[tokio::test]
async fn interrupted_date_rebuild_keeps_publication_and_cursor_uncommitted() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
    let date_text = date.to_string();
    let old = LongTermBucket {
        bucket_start_epoch: start_epoch,
        dimension: "model".to_string(),
        series_key: "model:last-good".to_string(),
        display_name: "last good".to_string(),
        reasoning_effort: String::new(),
        stats_date: Some(date_text.clone()),
        accumulator: LongTermAccumulator {
            calls: 41,
            ..LongTermAccumulator::default()
        },
    };
    let mut transaction = pool.begin().await.expect("seed transaction");
    insert_long_term_daily(&mut transaction, &old)
        .await
        .expect("seed last-good daily row");
    transaction
        .commit()
        .await
        .expect("commit last-good daily row");
    sqlx::query(
            "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'test')",
        )
        .bind(&date_text)
        .execute(&pool)
        .await
        .expect("seed dirty projection date");

    let mut daily = HashMap::new();
    for index in 0..=LONG_TERM_PROJECTION_WRITE_BATCH_ROWS {
        let series_key = format!("model:rebuilt-{index}");
        daily.insert(
            (date_text.clone(), "model".to_string(), series_key.clone()),
            LongTermBucket {
                bucket_start_epoch: start_epoch,
                dimension: "model".to_string(),
                series_key,
                display_name: "rebuilt".to_string(),
                reasoning_effort: String::new(),
                stats_date: Some(date_text.clone()),
                accumulator: LongTermAccumulator {
                    calls: 1,
                    ..LongTermAccumulator::default()
                },
            },
        );
    }
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let committed_batches = AtomicUsize::new(0);
    let control =
        LongTermProjectionWriteControl::cancelling_after(&shutdown, &gate, &committed_batches, 10);
    let rebuild = LongTermProjectionDateRebuild {
        bucket_date: date_text.clone(),
        start_epoch,
        end_epoch,
        hourly: HashMap::new(),
        daily,
        interval_segments: Vec::new(),
        source_row_count: 0,
    };
    commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        std::slice::from_ref(&rebuild),
        Some(321),
        &[LongTermProjectionDirtyBucket {
            bucket_date: date_text.clone(),
            generation: 1,
        }],
        false,
        &control,
    )
    .await
    .expect_err("cancellation before atomic publication");
    assert!(shutdown.is_cancelled());
    assert_eq!(committed_batches.load(Ordering::Acquire), 10);
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("cursor remains before publication"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("dirty date after interrupted handoff"),
        1
    );
    assert!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("active last-good backup")
            .is_some()
        );
    let public_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
        .await
        .expect("public last-good daily rows");
    assert_eq!(public_rows.len(), 1);
    assert_eq!(public_rows[0].series_key, "model:last-good");
    assert_eq!(public_rows[0].calls, 41);

    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        std::slice::from_ref(&rebuild),
        Some(321),
        &[LongTermProjectionDirtyBucket {
            bucket_date: date_text.clone(),
            generation: 1,
        }],
        false,
        &unrestricted,
    )
    .await
    .expect("resume atomically interrupted rebuild");
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("cursor after publication"),
        321
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
        )
        .bind(&date_text)
        .fetch_one(&pool)
        .await
        .expect("cleared dirty date"),
        0
    );
    assert_eq!(
            sqlx::query_scalar::<_, Option<String>>(
                "SELECT active_daily_backup_token FROM long_term_projection_bucket_state WHERE bucket_date = ?1",
            )
            .bind(&date_text)
            .fetch_one(&pool)
            .await
            .expect("released backup pointer"),
            None
        );
    let published_rows = load_long_term_daily_rows(&pool, "model", None, &date_text, &date_text)
        .await
        .expect("public rebuilt daily rows");
    assert_eq!(
        published_rows.len(),
        LONG_TERM_PROJECTION_WRITE_BATCH_ROWS + 1
    );
    assert!(
        published_rows
            .iter()
            .all(|row| row.series_key.starts_with("model:rebuilt-"))
    );
}

#[tokio::test]
async fn chunked_date_rebuild_keeps_empty_state_until_final_publication() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_EMPTY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("empty projection state");

    let first_date = NaiveDate::from_ymd_opt(2025, 1, 1).expect("first projection date");
    let first_date_text = first_date.to_string();
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls) VALUES (?1, 'model', 'model:last-good', 'last good', 41)",
        )
        .bind(&first_date_text)
        .execute(&pool)
        .await
        .expect("seed last-good first chunk row");
    let mut rebuilds = Vec::with_capacity(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES + 1);
    let mut dirty = Vec::with_capacity(LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES + 1);
    for offset in 0..=LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES {
        let date = first_date
            .checked_add_signed(ChronoDuration::days(offset as i64))
            .expect("projection date");
        let date_text = date.to_string();
        let (start_epoch, end_epoch) = long_term_day_epoch_bounds(date).expect("projection bounds");
        let mut daily = HashMap::new();
        if offset == 0 {
            daily.insert(
                (
                    date_text.clone(),
                    "model".to_string(),
                    "model:chunked".to_string(),
                ),
                LongTermBucket {
                    bucket_start_epoch: start_epoch,
                    dimension: "model".to_string(),
                    series_key: "model:chunked".to_string(),
                    display_name: "chunked".to_string(),
                    reasoning_effort: String::new(),
                    stats_date: Some(date_text.clone()),
                    accumulator: LongTermAccumulator {
                        calls: 1,
                        ..LongTermAccumulator::default()
                    },
                },
            );
        }
        sqlx::query(
                "INSERT INTO long_term_projection_dirty_buckets (bucket_date, repair_reason) VALUES (?1, 'chunked_test')",
            )
            .bind(&date_text)
            .execute(&pool)
            .await
            .expect("dirty projection date");
        rebuilds.push(LongTermProjectionDateRebuild {
            bucket_date: date_text.clone(),
            start_epoch,
            end_epoch,
            hourly: HashMap::new(),
            daily,
            interval_segments: Vec::new(),
            source_row_count: 0,
        });
        dirty.push(LongTermProjectionDirtyBucket {
            bucket_date: date_text,
            generation: 1,
        });
    }

    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let interrupted =
        LongTermProjectionWriteControl::stopping_after_rebuild_chunk(&shutdown, &gate);
    commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        &rebuilds,
        Some(321),
        &dirty,
        false,
        &interrupted,
    )
    .await
    .expect_err("second chunk must not start after cancellation");

    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("state before final publication");
    assert_eq!(status, LONG_TERM_STATUS_EMPTY);
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("cursor remains before final publication"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets",)
            .fetch_one(&pool)
            .await
            .expect("all dirty dates remain before publication"),
        (LONG_TERM_PROJECTION_REBUILD_PUBLICATION_DATES + 1) as i64
    );
    let visible_rows =
        load_long_term_daily_rows(&pool, "model", None, &first_date_text, &first_date_text)
            .await
            .expect("staged chunk keeps last-good public row");
    assert_eq!(visible_rows.len(), 1);
    assert_eq!(visible_rows[0].series_key, "model:last-good");
    assert_eq!(visible_rows[0].calls, 41);

    let unrestricted = LongTermProjectionWriteControl::unrestricted();
    commit_long_term_projection_date_rebuilds_with_control(
        &pool,
        &rebuilds,
        Some(321),
        &dirty,
        false,
        &unrestricted,
    )
    .await
    .expect("resume final publication");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("state after final publication");
    assert_eq!(status, LONG_TERM_STATUS_READY);
    assert_eq!(
        load_long_term_projection_cursor(&pool)
            .await
            .expect("cursor after final publication"),
        321
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets",)
            .fetch_one(&pool)
            .await
            .expect("cleared dirty chunks"),
        0
    );
    assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM long_term_projection_date_publications",
            )
            .fetch_one(&pool)
            .await
            .expect("completed publication metadata is pruned"),
            0
        );
    let published_rows =
        load_long_term_daily_rows(&pool, "model", None, &first_date_text, &first_date_text)
            .await
            .expect("published first chunk row");
    assert_eq!(published_rows.len(), 1);
    assert_eq!(published_rows[0].series_key, "model:chunked");
}

#[tokio::test]
async fn projection_date_rebuild_standard_timestamp_branch_uses_occurred_at_seek() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    create_long_term_test_invocations(&pool).await;
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query(
        "CREATE INDEX idx_codex_invocations_occurred_at ON codex_invocations (occurred_at)",
    )
    .execute(&pool)
    .await
    .expect("occurred_at index");
    let query = long_term_projection_canonical_query("SELECT inv.id FROM codex_invocations inv");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!("EXPLAIN QUERY PLAN {query}"))
        .bind("2026-07-26 00:00:00")
        .bind("2026-07-27 00:00:00")
        .fetch_all(&pool)
        .await
        .expect("query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_codex_invocations_occurred_at")
            && detail.contains("occurred_at>? AND occurred_at<?")
    }));
}

#[tokio::test]
async fn projection_date_rebuild_skips_rfc3339_fallback_for_canonical_live_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    create_long_term_test_invocations(&pool).await;
    ensure_long_term_projection_source_indexes(&pool)
        .await
        .expect("projection source indexes");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (1, 'canonical', '2026-07-26 12:00:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("canonical invocation");

    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("projection date");
    let start = Shanghai
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("day start"))
        .single()
        .expect("Shanghai day start");
    let end = Shanghai
        .from_local_datetime(
            &date
                .succ_opt()
                .expect("next date")
                .and_hms_opt(0, 0, 0)
                .expect("day end"),
        )
        .single()
        .expect("Shanghai day end");
    assert!(
        load_long_term_projection_live_rfc3339_compatibility(&pool)
            .await
            .expect("canonical compatibility gate")
            .is_none()
    );
    let control = LongTermProjectionWriteControl::unrestricted();
    let canonical_rows = load_long_term_projection_rows_for_date(&pool, date, start, end, &control)
        .await
        .expect("canonical projection rows");
    assert_eq!(
        canonical_rows
            .iter()
            .filter_map(|row| row.invoke_id.as_deref())
            .collect::<HashSet<_>>(),
        HashSet::from(["canonical"])
    );

    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (2, 'rfc3339', '2026-07-25T16:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (3, 'rfc3339-negative-offset', '2026-07-25T02:00:01-14:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 negative-offset invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (4, 'rfc3339-historical', '2020-01-01T00:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("historical RFC3339 invocation");
    let rfc3339_compatibility = load_long_term_projection_live_rfc3339_compatibility(&pool)
        .await
        .expect("updated RFC3339 compatibility gate")
        .expect("RFC3339 compatibility metadata");
    let (rfc3339_lower, rfc3339_upper) =
        long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
    assert_eq!(rfc3339_lower, "2026-07-25T01:59:59");
    let query = long_term_projection_live_rfc3339_query("SELECT inv.id FROM codex_invocations inv");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!("EXPLAIN QUERY PLAN {query}"))
        .bind(&rfc3339_lower)
        .bind(&rfc3339_upper)
        .bind(start.timestamp())
        .bind(end.timestamp())
        .fetch_all(&pool)
        .await
        .expect("RFC3339 range query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_codex_invocations_long_term_projection_rfc3339_occurred_at")
            && detail.contains("occurred_at>? AND occurred_at<?")
    }));
    let mixed_rows = load_long_term_projection_rows_for_date(&pool, date, start, end, &control)
        .await
        .expect("mixed projection rows");
    assert_eq!(
        mixed_rows
            .iter()
            .filter_map(|row| row.invoke_id.as_deref())
            .collect::<HashSet<_>>(),
        HashSet::from(["canonical", "rfc3339", "rfc3339-negative-offset"])
    );
}
