#[tokio::test]
async fn invocation_correction_marks_rfc3339_fractional_cross_day_buckets() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    ensure_long_term_projection_correction_trigger(&pool)
        .await
        .expect("correction trigger");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model, t_total_ms) VALUES (1, '2026-07-25T15:59:59.500Z', 'success', 'before', 600)",
        )
        .execute(&pool)
        .await
        .expect("fractional RFC3339 invocation");

    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("historical correction");
    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("dirty correction dates");
    assert_eq!(dirty_dates, vec!["2026-07-25", "2026-07-26"]);

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
        .execute(&pool)
        .await
        .expect("clear fractional correction markers");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (2, '2026-07-25T16:00:00Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 day-start invocation");
    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 2")
        .execute(&pool)
        .await
        .expect("RFC3339 day-start correction");
    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("day-start correction dates");
    assert_eq!(dirty_dates, vec!["2026-07-26"]);

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
        .execute(&pool)
        .await
        .expect("clear day-start correction markers");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (3, '2026-07-25T15:59:59.9999Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-start invocation");
    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 3")
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-start correction");
    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("sub-millisecond pre-start correction dates");
    assert_eq!(dirty_dates, vec!["2026-07-25"]);

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
        .execute(&pool)
        .await
        .expect("clear cross-day correction markers");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model, t_total_ms) VALUES (4, '2026-07-25T15:59:59.9999Z', 'success', 'before', 1)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond cross-day invocation");
    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 4")
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond cross-day correction");
    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("cross-day correction dates");
    assert_eq!(dirty_dates, vec!["2026-07-25", "2026-07-26"]);

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
        .execute(&pool)
        .await
        .expect("clear sub-microsecond correction markers");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (5, '2026-07-25T15:59:59.9999999Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-start invocation");
    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 5")
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-start correction");
    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("sub-microsecond pre-start correction dates");
    assert_eq!(dirty_dates, vec!["2026-07-25"]);

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
        .execute(&pool)
        .await
        .expect("clear high-precision correction markers");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (6, '2026-07-25T15:59:59.99999999999999999999Z', 'success', 'before')",
        )
        .execute(&pool)
        .await
        .expect("high-precision pre-start invocation");
    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 6")
        .execute(&pool)
        .await
        .expect("high-precision pre-start correction");
    let dirty_dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("high-precision correction dates");
    assert_eq!(dirty_dates, vec!["2026-07-25"]);
}

#[tokio::test]
async fn archive_compatibility_cache_is_checksum_and_opened_bytes_scoped() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    let control = LongTermProjectionWriteControl::unrestricted();
    let original = LongTermArchiveCompatibility {
        has_legacy_crossing: true,
        legacy_max_duration_ms: Some(2_000.0),
        legacy_min_occurred_at: Some("2026-07-25 23:59:59".to_string()),
        has_rfc3339: false,
        rfc3339_max_duration_ms: None,
        rfc3339_min_occurred_at: None,
    };
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-long-term-archive-fingerprint-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    std::fs::write(&archive_path, b"canonical").expect("write original same-sized archive bytes");
    let original_metadata = std::fs::metadata(&archive_path).expect("original archive metadata");
    let original_mtime = original_metadata
        .modified()
        .expect("original archive mtime");
    let original_fingerprint =
        long_term_archive_file_fingerprint(archive_path.to_str().expect("UTF-8 archive path"))
            .expect("fingerprint original archive bytes");
    let archive_file_path = archive_path.to_string_lossy().to_string();
    persist_long_term_archive_compatibility(
        &pool,
        &archive_file_path,
        "archive-sha-one",
        &original_fingerprint,
        original.clone(),
        &control,
    )
    .await
    .expect("persist archive capability");
    assert_eq!(
        load_long_term_archive_compatibility(
            &pool,
            &archive_file_path,
            "archive-sha-one",
            &original_fingerprint,
        )
        .await
        .expect("load matching capability"),
        Some(original)
    );
    assert_eq!(
        load_long_term_archive_compatibility(
            &pool,
            &archive_file_path,
            "archive-sha-two",
            &original_fingerprint,
        )
        .await
        .expect("reject stale capability"),
        None
    );
    std::fs::write(&archive_path, b"rfc-3333!")
        .expect("write replacement same-sized archive bytes");
    filetime::set_file_mtime(
        &archive_path,
        filetime::FileTime::from_system_time(original_mtime),
    )
    .expect("restore replacement archive mtime");
    assert_eq!(
        std::fs::metadata(&archive_path)
            .expect("replacement archive metadata")
            .len(),
        original_metadata.len()
    );
    let replacement_fingerprint =
        long_term_archive_file_fingerprint(archive_path.to_str().expect("UTF-8 archive path"))
            .expect("fingerprint replacement archive bytes");
    assert_ne!(replacement_fingerprint, original_fingerprint);
    assert_eq!(
        load_long_term_archive_compatibility(
            &pool,
            &archive_file_path,
            "archive-sha-one",
            &replacement_fingerprint,
        )
        .await
        .expect("reject replaced archive capability"),
        None
    );
    std::fs::remove_file(&archive_path).expect("remove temporary archive file");
}

#[tokio::test]
async fn archive_pool_fingerprint_uses_the_opened_database_bytes() {
    let unique = format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    );
    let archive_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-long-term-fingerprint-source-{unique}.sqlite.gz"
    ));
    let opened_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-long-term-fingerprint-opened-{unique}.sqlite"
    ));
    std::fs::write(
        &archive_path,
        b"archive bytes that are not the opened database",
    )
    .expect("write archive source bytes");
    fs::File::create(&opened_path).expect("create opened archive database");
    let options = format!("sqlite://{}", opened_path.to_string_lossy())
        .parse::<SqliteConnectOptions>()
        .expect("parse opened archive database URL")
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("open materialized archive database");
    sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("write materialized archive database");

    let opened_fingerprint = long_term_archive_pool_fingerprint(&pool)
        .await
        .expect("fingerprint opened archive database");
    assert_eq!(
        opened_fingerprint,
        long_term_archive_file_fingerprint(opened_path.to_str().expect("UTF-8 opened path"))
            .expect("fingerprint opened archive path")
    );
    assert_ne!(
        opened_fingerprint,
        long_term_archive_file_fingerprint(archive_path.to_str().expect("UTF-8 archive path"))
            .expect("fingerprint archive source path")
    );

    pool.close().await;
    std::fs::remove_file(&archive_path).expect("remove archive source file");
    std::fs::remove_file(&opened_path).expect("remove opened archive database");
}

#[tokio::test]
async fn archive_compatibility_inspection_checks_cancellation_between_bounded_batches() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
    sqlx::query(
        r#"
            WITH RECURSIVE source(id) AS (
                VALUES(1)
                UNION ALL
                SELECT id + 1 FROM source WHERE id < 1025
            )
            INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms)
            SELECT
                id,
                printf('2026-07-%02d 00:00:00', (id % 28) + 1),
                CASE WHEN id = 1025 THEN 'success' ELSE 'running' END,
                1
            FROM source
            "#,
    )
    .execute(&pool)
    .await
    .expect("archive source rows");
    let query = long_term_archive_invocation_query_for_range(&pool)
        .await
        .expect("archive range queries");
    let shutdown = CancellationToken::new();
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::ZERO);
    let interrupted = LongTermProjectionWriteControl::stopping_after_archive_compatibility_batch(
        &shutdown, &gate,
    );
    let error = inspect_long_term_archive_compatibility(&pool, &query.parts, &interrupted)
        .await
        .expect_err("cancellation stops before the second bounded scan");
    assert!(long_term_projection_write_is_deferred(&error));

    let recovered_control = LongTermProjectionWriteControl::unrestricted();
    let recovered =
        inspect_long_term_archive_compatibility(&pool, &query.parts, &recovered_control)
            .await
            .expect("a later attempt rescans and recovers compatibility");
    assert!(recovered.has_legacy_crossing);
    assert_eq!(recovered.legacy_max_duration_ms, Some(1.0));
}

#[tokio::test]
async fn archive_compatibility_inspection_includes_the_minimum_rowid() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (?1, '2026-07-25T02:00:01-14:00', 'success')",
        )
        .bind(i64::MIN)
        .execute(&pool)
        .await
        .expect("minimum rowid archive invocation");
    let query = long_term_archive_invocation_query_for_range(&pool)
        .await
        .expect("archive range queries");
    let control = LongTermProjectionWriteControl::unrestricted();
    let compatibility = inspect_long_term_archive_compatibility(&pool, &query.parts, &control)
        .await
        .expect("archive compatibility probe");
    assert!(compatibility.has_rfc3339);
    assert_eq!(
        compatibility.rfc3339_min_occurred_at.as_deref(),
        Some("2026-07-25T02:00:01-14:00")
    );
}

#[tokio::test]
async fn archive_range_query_keeps_fractional_rfc3339_cross_day_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
    sqlx::query(
        "CREATE INDEX idx_archive_invocations_occurred_at ON codex_invocations (occurred_at)",
    )
    .execute(&pool)
    .await
    .expect("archive occurred_at index");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (1, '2026-07-25T15:59:59.500Z', 'success', 600)",
        )
        .execute(&pool)
        .await
        .expect("archive fractional RFC3339 row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (2, '2026-07-25T16:00:00Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 day-start row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (3, '2026-07-26T16:00:00Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 next-day-start row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (4, '2026-07-25T15:59:59.9999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-millisecond pre-start row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (5, '2026-07-26T15:59:59.9999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-millisecond pre-end row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (6, '2026-07-25T15:59:59.9999999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-microsecond pre-start row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (7, '2026-07-26T15:59:59.9999999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 sub-microsecond pre-end row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (8, '2026-07-25T15:59:59.9999999Z', 'success', 0.001)",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 nanosecond crossing row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (9, '2026-07-25T15:59:59.99999999999999999999Z', 'success')",
        )
        .execute(&pool)
        .await
        .expect("archive high-precision RFC3339 pre-start row");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (10, '2026-07-25T02:00:01-14:00', 'success', 2000)",
        )
        .execute(&pool)
        .await
        .expect("archive RFC3339 negative-offset row");
    let queries = long_term_archive_invocation_query_for_range(&pool)
        .await
        .expect("archive range queries");
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
                .expect("next day start"),
        )
        .single()
        .expect("Shanghai next day start");
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
        "EXPLAIN QUERY PLAN {}",
        queries.canonical
    ))
    .bind(&start_text)
    .bind(&end_text)
    .fetch_all(&pool)
    .await
    .expect("canonical archive query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_archive_invocations_occurred_at")
            && detail.contains("occurred_at>? AND occurred_at<?")
    }));

    let control = LongTermProjectionWriteControl::unrestricted();
    let compatibility = inspect_long_term_archive_compatibility(&pool, &queries.parts, &control)
        .await
        .expect("archive compatibility probe");
    assert_eq!(
        compatibility,
        LongTermArchiveCompatibility {
            has_legacy_crossing: false,
            legacy_max_duration_ms: None,
            legacy_min_occurred_at: None,
            has_rfc3339: true,
            rfc3339_max_duration_ms: Some(2_000.0),
            rfc3339_min_occurred_at: Some("2026-07-25T02:00:01-14:00".to_string()),
        }
    );
    let rfc3339_compatibility = LongTermRfc3339Compatibility {
        max_duration_ms: compatibility.rfc3339_max_duration_ms,
    };
    let (rfc3339_lower, rfc3339_upper) =
        long_term_rfc3339_text_bounds(start, end, &rfc3339_compatibility);
    let rfc3339_plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
        "EXPLAIN QUERY PLAN {}",
        queries.rfc3339
    ))
    .bind(&rfc3339_lower)
    .bind(&rfc3339_upper)
    .bind(start.timestamp())
    .bind(end.timestamp())
    .fetch_all(&pool)
    .await
    .expect("RFC3339 archive query plan");
    assert!(rfc3339_plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_archive_invocations_occurred_at")
            && detail.contains("occurred_at>? AND occurred_at<?")
    }));
    let rows = load_long_term_archive_invocation_rows_for_range(
        &pool,
        &queries,
        compatibility,
        start,
        end,
    )
    .await
    .expect("archive range rows");
    let ids = rows.into_iter().map(|row| row.id).collect::<Vec<_>>();
    assert_eq!(ids, vec![1, 2, 5, 7, 8, 10]);
}

#[tokio::test]
async fn archive_range_bounds_legacy_crossing_with_cached_max_duration() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT, t_total_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
    sqlx::query("CREATE INDEX idx_archive_legacy_occurred_at ON codex_invocations (occurred_at)")
        .execute(&pool)
        .await
        .expect("archive occurred_at index");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, t_total_ms) VALUES (1, '2026-01-01 00:00:00', 'success', 2000), (2, '2026-07-25 23:59:59', 'success', 2000), (3, '2026-07-26 12:00:00', 'success', 100), (4, '2026-02-01 00:00:00', 'success', 1e300)",
        )
        .execute(&pool)
        .await
        .expect("canonical archive rows");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    let queries = long_term_archive_invocation_query_for_range(&pool)
        .await
        .expect("archive range queries");
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
                .expect("next day start"),
        )
        .single()
        .expect("Shanghai next day start");
    let control = LongTermProjectionWriteControl::unrestricted();
    let compatibility = inspect_long_term_archive_compatibility(&pool, &queries.parts, &control)
        .await
        .expect("archive compatibility probe");
    assert_eq!(
        compatibility,
        LongTermArchiveCompatibility {
            has_legacy_crossing: true,
            legacy_max_duration_ms: Some(1e300),
            legacy_min_occurred_at: Some("2026-01-01 00:00:00".to_string()),
            has_rfc3339: false,
            rfc3339_max_duration_ms: None,
            rfc3339_min_occurred_at: None,
        }
    );
    assert!(long_term_archive_legacy_crossing_start(&start, 1e300).is_none());
    let crossing_start = compatibility
        .legacy_min_occurred_at
        .as_deref()
        .expect("cached bounded legacy start");
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
        "EXPLAIN QUERY PLAN {}",
        queries.crossing_text
    ))
    .bind(crossing_start)
    .bind(&start_text)
    .fetch_all(&pool)
    .await
    .expect("bounded legacy crossing query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_archive_legacy_occurred_at")
            && detail.contains("occurred_at>? AND occurred_at<?")
    }));

    persist_long_term_archive_compatibility(
        &pool,
        "legacy.sqlite.gz",
        "legacy-sha",
        "legacy-fingerprint",
        compatibility,
        &control,
    )
    .await
    .expect("persist legacy compatibility");
    let cached = load_long_term_archive_compatibility(
        &pool,
        "legacy.sqlite.gz",
        "legacy-sha",
        "legacy-fingerprint",
    )
    .await
    .expect("load legacy compatibility")
    .expect("cached legacy compatibility");
    let rows =
        load_long_term_archive_invocation_rows_for_range(&pool, &queries, cached, start, end)
            .await
            .expect("bounded legacy archive rows");
    assert_eq!(
        rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
}

#[tokio::test]
async fn archive_range_uses_cached_canonical_capability_without_a_scan() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, status TEXT)",
        )
        .execute(&pool)
        .await
        .expect("archive invocation schema");
    sqlx::query("CREATE INDEX idx_archive_standard_occurred_at ON codex_invocations (occurred_at)")
        .execute(&pool)
        .await
        .expect("archive occurred_at index");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status) VALUES (1, '2026-07-26 12:00:00', 'success')",
        )
        .execute(&pool)
        .await
        .expect("standard archive row");
    let queries = long_term_archive_invocation_query_for_range(&pool)
        .await
        .expect("archive range queries");
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
                .expect("next day start"),
        )
        .single()
        .expect("Shanghai next day start");
    let start_text = start.format("%Y-%m-%d %H:%M:%S").to_string();
    let end_text = end.format("%Y-%m-%d %H:%M:%S").to_string();
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
        "EXPLAIN QUERY PLAN {}",
        queries.canonical
    ))
    .bind(&start_text)
    .bind(&end_text)
    .fetch_all(&pool)
    .await
    .expect("canonical archive query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_archive_standard_occurred_at")
            && detail.contains("occurred_at>? AND occurred_at<?")
    }));
    let rows = load_long_term_archive_invocation_rows_for_range(
        &pool,
        &queries,
        LongTermArchiveCompatibility {
            has_legacy_crossing: false,
            legacy_max_duration_ms: None,
            legacy_min_occurred_at: None,
            has_rfc3339: false,
            rfc3339_max_duration_ms: None,
            rfc3339_min_occurred_at: None,
        },
        start,
        end,
    )
    .await
    .expect("standard archive range rows");
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), vec![1]);
}

#[tokio::test]
async fn archive_cleanup_safe_starts_preserve_nanosecond_cross_day_endpoints() {
    let occurred_at = "2026-07-25T15:59:59.9999999Z";
    let expected_safe_start = NaiveDate::from_ymd_opt(2026, 7, 27).expect("fixed date");
    let unique = format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let invocation_db_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-invocation-boundary-{unique}.sqlite"
    ));
    let invocation_archive_path = invocation_db_path.with_extension("sqlite.gz");
    fs::File::create(&invocation_db_path).expect("create invocation archive database");
    let invocation_options = format!("sqlite://{}", invocation_db_path.to_string_lossy())
        .parse::<SqliteConnectOptions>()
        .expect("parse invocation archive URL")
        .create_if_missing(true);
    let invocation_archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(invocation_options)
        .await
        .expect("open invocation archive database");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, occurred_at TEXT NOT NULL, t_total_ms REAL)",
        )
        .execute(&invocation_archive_pool)
        .await
        .expect("create invocation archive schema");
    sqlx::query(
        "INSERT INTO codex_invocations (id, occurred_at, t_total_ms) VALUES (1, ?1, 0.001)",
    )
    .bind(occurred_at)
    .execute(&invocation_archive_pool)
    .await
    .expect("insert nanosecond invocation source");
    invocation_archive_pool.close().await;
    crate::maintenance::deflate_sqlite_file_to_gzip(&invocation_db_path, &invocation_archive_path)
        .expect("compress invocation archive");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("full schema");
    let invocation_archive_sha256 = crate::maintenance::sha256_hex_file(&invocation_archive_path)
        .expect("hash invocation archive");
    sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('codex_invocations', '2026-07', ?1, ?2, 1, 'completed', datetime('now'))",
        )
        .bind(invocation_archive_path.to_string_lossy().to_string())
        .bind(&invocation_archive_sha256)
        .execute(&pool)
        .await
        .expect("record invocation archive manifest");
    assert_eq!(
        long_term_integrity_source_safe_start_for_archive_cleanup(
            &pool,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            invocation_archive_path.to_string_lossy().as_ref(),
            None,
        )
        .await
        .expect("read invocation archive boundary"),
        Some(expected_safe_start)
    );
    sqlx::query("UPDATE archive_batches SET sha256 = 'stale-invocation-sha' WHERE file_path = ?1")
        .bind(invocation_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("stale invocation manifest");
    assert!(
        long_term_integrity_source_safe_start_for_archive_cleanup(
            &pool,
            HOURLY_ROLLUP_DATASET_INVOCATIONS,
            invocation_archive_path.to_string_lossy().as_ref(),
            None,
        )
        .await
        .expect_err("mismatched invocation archive must not define a cleanup boundary")
        .to_string()
        .contains("does not match")
    );
    sqlx::query("UPDATE archive_batches SET sha256 = ?1 WHERE file_path = ?2")
        .bind(&invocation_archive_sha256)
        .bind(invocation_archive_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("restore invocation manifest");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, payload, raw_response, t_total_ms) VALUES (1, 'boundary-invoke', ?1, 'success', '{}', '{}', 0.001)",
        )
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("insert live invocation source");

    let attempt_db_path = std::env::temp_dir().join(format!(
        "codex-vibe-monitor-attempt-boundary-{unique}.sqlite"
    ));
    let attempt_archive_path = attempt_db_path.with_extension("sqlite.gz");
    fs::File::create(&attempt_db_path).expect("create attempt archive database");
    let attempt_options = format!("sqlite://{}", attempt_db_path.to_string_lossy())
        .parse::<SqliteConnectOptions>()
        .expect("parse attempt archive URL")
        .create_if_missing(true);
    let attempt_archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(attempt_options)
        .await
        .expect("open attempt archive database");
    sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, upstream_account_id INTEGER)",
        )
        .execute(&attempt_archive_pool)
        .await
        .expect("create attempt archive schema");
    sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, upstream_account_id) VALUES (1, 'boundary-invoke', ?1, 42)",
        )
        .bind(occurred_at)
        .execute(&attempt_archive_pool)
        .await
        .expect("insert attempt mapping");
    attempt_archive_pool.close().await;
    crate::maintenance::deflate_sqlite_file_to_gzip(&attempt_db_path, &attempt_archive_path)
        .expect("compress attempt archive");
    let attempt_archive_sha256 =
        crate::maintenance::sha256_hex_file(&attempt_archive_path).expect("hash attempt archive");
    sqlx::query(
            "INSERT INTO archive_batches (dataset, month_key, file_path, sha256, row_count, status, created_at) VALUES ('pool_upstream_request_attempts', '2026-07', ?1, ?2, 1, 'completed', datetime('now'))",
        )
        .bind(attempt_archive_path.to_string_lossy().to_string())
        .bind(&attempt_archive_sha256)
        .execute(&pool)
        .await
        .expect("record attempt archive manifest");
    assert_eq!(
        long_term_integrity_source_safe_start_for_archive_cleanup(
            &pool,
            "pool_upstream_request_attempts",
            attempt_archive_path.to_string_lossy().as_ref(),
            None,
        )
        .await
        .expect("read attempt archive boundary"),
        Some(expected_safe_start)
    );

    for path in [
        invocation_db_path,
        invocation_archive_path,
        attempt_db_path,
        attempt_archive_path,
    ] {
        let _ = fs::remove_file(path);
    }
}

#[tokio::test]
async fn invocation_correction_wakes_a_deferred_projection_repair() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    ensure_long_term_projection_correction_trigger(&pool)
        .await
        .expect("correction trigger");
    sqlx::query("INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (1, '2026-07-26 10:00:00', 'success', 'before')")
            .execute(&pool)
            .await
            .expect("invocation");
    queue_long_term_projection_repairs(&pool, &["2026-07-26".to_string()], "source_unavailable")
        .await
        .expect("dirty bucket");
    defer_long_term_projection_repair(&pool, "2026-07-26")
        .await
        .expect("defer repair");

    sqlx::query("UPDATE codex_invocations SET model = 'after' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("correct invocation");

    let next_attempt_at = sqlx::query_scalar::<_, Option<String>>(
            "SELECT next_attempt_at FROM long_term_projection_dirty_buckets WHERE bucket_date = '2026-07-26'",
        )
        .fetch_one(&pool)
        .await
        .expect("repair marker");
    assert!(next_attempt_at.is_none());
}

#[tokio::test]
async fn terminal_finalize_does_not_enqueue_a_long_term_date_rebuild() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    ensure_long_term_projection_correction_trigger(&pool)
        .await
        .expect("correction trigger");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (1, '2026-07-26 10:00:00', 'running', 'gpt-5')",
        )
        .execute(&pool)
        .await
        .expect("running invocation");

    sqlx::query("UPDATE codex_invocations SET status = 'success' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("terminal finalize");
    let dirty_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&pool)
            .await
            .expect("dirty count");
    assert_eq!(dirty_count, 0);

    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, failure_kind, model) VALUES (2, '2026-07-26 11:00:00', 'interrupted', 'proxy_interrupted', 'gpt-5')",
        )
        .execute(&pool)
        .await
        .expect("recoverable interrupted invocation");
    sqlx::query(
        "UPDATE codex_invocations SET status = 'success', failure_kind = NULL WHERE id = 2",
    )
    .execute(&pool)
    .await
    .expect("recovered terminal finalize");
    let dirty_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&pool)
            .await
            .expect("dirty count");
    assert_eq!(dirty_count, 0);

    sqlx::query("UPDATE codex_invocations SET model = 'gpt-5.1' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("historical correction");
    let dirty_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&pool)
            .await
            .expect("dirty count");
    assert_eq!(dirty_count, 1);
}

#[tokio::test]
async fn out_of_order_terminal_finalize_queues_an_exact_date_rebuild() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, source TEXT, status TEXT, occurred_at TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, t_total_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, t_upstream_stream_ms REAL, error_message TEXT, failure_kind TEXT)",
        )
        .execute(&pool)
        .await
        .expect("invocation schema");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    ensure_long_term_projection_correction_trigger(&pool)
        .await
        .expect("correction trigger");
    sqlx::query(
            "INSERT INTO codex_invocations (id, occurred_at, status, model) VALUES (1, '2026-07-26 10:00:00', 'running', 'gpt-5'), (2, '2026-07-26 11:00:00', 'success', 'gpt-5')",
        )
        .execute(&pool)
        .await
        .expect("out-of-order source rows");
    sqlx::query("INSERT INTO long_term_projection_state (consumer, cursor_row_id) VALUES (?1, 2)")
        .bind(LONG_TERM_PROJECTION_CONSUMER)
        .execute(&pool)
        .await
        .expect("advanced projection cursor");

    sqlx::query("UPDATE codex_invocations SET status = 'success' WHERE id = 1")
        .execute(&pool)
        .await
        .expect("late terminal finalize");
    let dirty_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_projection_dirty_buckets")
            .fetch_one(&pool)
            .await
            .expect("dirty count");
    assert_eq!(dirty_count, 1);
}

#[tokio::test]
async fn cursor_repair_defer_applies_to_every_affected_date() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    let dates = vec!["2026-07-26".to_string(), "2026-07-27".to_string()];
    queue_long_term_projection_repairs(&pool, &dates, "interval_baseline")
        .await
        .expect("dirty buckets");

    defer_long_term_projection_repairs(&pool, &dates)
        .await
        .expect("defer repairs");
    ensure_long_term_projection_repairs(&pool, &dates, "interval_baseline")
        .await
        .expect("ensure existing repairs");
    assert!(
        long_term_projection_repairs_are_deferred(&pool, &dates)
            .await
            .expect("load repair deadline")
    );

    let deferred = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_projection_dirty_buckets WHERE next_attempt_at IS NOT NULL AND datetime(next_attempt_at) > datetime('now')",
        )
        .fetch_one(&pool)
        .await
        .expect("deferred markers");
    assert_eq!(deferred, 2);
}

#[tokio::test]
async fn archive_trigger_invalidates_month_when_coverage_is_unknown() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    sqlx::query(
            "CREATE TABLE archive_batches (id INTEGER PRIMARY KEY, dataset TEXT NOT NULL, month_key TEXT NOT NULL, file_path TEXT NOT NULL, sha256 TEXT NOT NULL, status TEXT NOT NULL, coverage_start_at TEXT, coverage_end_at TEXT, historical_rollups_materialized_at TEXT)",
        )
        .execute(&pool)
        .await
        .expect("archive schema");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    ensure_long_term_projection_archive_trigger(&pool)
        .await
        .expect("archive trigger");

    sqlx::query(
            "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, status) VALUES (1, 'codex_invocations', '2026-07', 'legacy.db', 'sha', 'completed')",
        )
        .execute(&pool)
        .await
        .expect("unknown coverage archive");

    let dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("dirty dates");
    assert_eq!(dates.len(), 31);
    assert_eq!(dates.first().map(String::as_str), Some("2026-07-01"));
    assert_eq!(dates.last().map(String::as_str), Some("2026-07-31"));

    sqlx::query("DELETE FROM long_term_projection_dirty_buckets")
        .execute(&pool)
        .await
        .expect("clear month fallback markers");
    sqlx::query(
            "INSERT INTO archive_batches (id, dataset, month_key, file_path, sha256, status, coverage_start_at, coverage_end_at) VALUES (2, 'codex_invocations', '2026-07', 'fractional.db', 'fractional-sha', 'completed', '2026-07-25T15:59:59.9999Z', '2026-07-25T16:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("fractional RFC3339 archive coverage");
    let dates = sqlx::query_scalar::<_, String>(
        "SELECT bucket_date FROM long_term_projection_dirty_buckets ORDER BY bucket_date",
    )
    .fetch_all(&pool)
    .await
    .expect("fractional archive dirty dates");
    assert_eq!(dates, vec!["2026-07-25", "2026-07-26"]);
}

#[tokio::test]
async fn accepted_projection_baseline_recovers_the_public_stats_state() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_RUNNING)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("interrupted state");

    commit_long_term_projection_date_rebuilds(&pool, &[], Some(7), &[], true)
        .await
        .expect("accepted baseline");

    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("recovered state");
    assert_eq!(status, LONG_TERM_STATUS_READY);

    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("integrity error state");
    commit_long_term_projection_date_rebuilds(&pool, &[], Some(8), &[], false)
        .await
        .expect("baseline while error is retained");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("retained error state");
    assert_eq!(status, LONG_TERM_STATUS_ERROR);
}

#[tokio::test]
async fn nonempty_projection_repair_recovers_empty_state_and_advances_start_date() {
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
            "UPDATE long_term_stats_state SET status = ?1, statistics_start_date = '2026-07-27' WHERE id = ?2",
        )
        .bind(LONG_TERM_STATUS_EMPTY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("empty state");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (1, 'first-repaired', '2026-07-26 10:00:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("first repaired invocation");

    let control = LongTermProjectionWriteControl::unrestricted();
    let rebuild = build_long_term_projection_date_rebuild(&pool, "2026-07-26", &control)
        .await
        .expect("date rebuild");
    commit_long_term_projection_date_rebuilds(&pool, &[rebuild], Some(1), &[], false)
        .await
        .expect("repair commit");

    let (status, start_date) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("repaired state");
    assert_eq!(status, LONG_TERM_STATUS_READY);
    assert_eq!(start_date.as_deref(), Some("2026-07-26"));
}

#[tokio::test]
async fn incremental_projection_preserves_an_existing_error_state() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query(
            "UPDATE long_term_stats_state SET status = ?1, last_error = 'source unavailable' WHERE id = ?2",
        )
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("error state");
    queue_long_term_projection_repairs(&pool, &["2026-07-25".to_string()], "source_unavailable")
        .await
        .expect("dirty bucket");
    let runtime = Arc::new(Mutex::new(LongTermProjectionRuntime::default()));

    apply_long_term_projection_incremental_with_runtime(
        &pool,
        &runtime,
        &HashMap::new(),
        &HashMap::new(),
        &[],
        7,
        1,
    )
    .await
    .expect("incremental flush");

    let (status, last_error) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, last_error FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("preserved state");
    assert_eq!(status, LONG_TERM_STATUS_ERROR);
    assert_eq!(last_error.as_deref(), Some("source unavailable"));
}

#[tokio::test]
async fn account_kind_change_invalidates_affected_projection_dates() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    create_long_term_test_invocations(&pool).await;
    sqlx::query("CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY, kind TEXT)")
        .execute(&pool)
        .await
        .expect("account schema");
    sqlx::query("CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT, upstream_account_id INTEGER)")
            .execute(&pool)
            .await
            .expect("attempt schema");
    sqlx::query("CREATE TABLE archive_batches (id INTEGER PRIMARY KEY, dataset TEXT NOT NULL, month_key TEXT NOT NULL, status TEXT NOT NULL, coverage_start_at TEXT, coverage_end_at TEXT)")
            .execute(&pool)
            .await
            .expect("archive schema");
    ensure_long_term_projection_schema(&pool)
        .await
        .expect("projection schema");
    ensure_long_term_projection_account_trigger(&pool)
        .await
        .expect("account trigger");
    sqlx::query("INSERT INTO pool_upstream_accounts (id, kind) VALUES (42, 'oauth_codex')")
        .execute(&pool)
        .await
        .expect("account");
    sqlx::query("INSERT INTO archive_batches (id, dataset, month_key, status, coverage_start_at, coverage_end_at) VALUES (1, 'codex_invocations', '2026-06', 'completed', '2026-06-01 00:00:00', '2026-06-02 23:59:59')")
            .execute(&pool)
            .await
            .expect("archived coverage");
    sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, payload) VALUES (1, 'classified-before-day-start', '2026-07-25T15:59:59.9999Z', 'success', 100, '{\"upstreamAccountId\":42}')")
            .execute(&pool)
            .await
            .expect("classified sub-millisecond pre-start invocation");
    sqlx::query("INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, payload) VALUES (2, 'classified-before-next-day-start', '2026-07-26T15:59:59.9999Z', 'success', 100, '{\"upstreamAccountId\":42}')")
            .execute(&pool)
            .await
            .expect("classified sub-millisecond pre-end invocation");

    sqlx::query("UPDATE pool_upstream_accounts SET kind = 'api_key' WHERE id = 42")
        .execute(&pool)
        .await
        .expect("classification update");

    let active_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets WHERE bucket_date LIKE '2026-07-%' ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("affected dates");
    assert_eq!(active_dates, vec!["2026-07-25", "2026-07-26"]);
    let archived_dates = sqlx::query_scalar::<_, String>(
            "SELECT bucket_date FROM long_term_projection_dirty_buckets WHERE bucket_date LIKE '2026-06-%' ORDER BY bucket_date",
        )
        .fetch_all(&pool)
        .await
        .expect("archived dates");
    assert_eq!(archived_dates, vec!["2026-06-01", "2026-06-02"]);
}
