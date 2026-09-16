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

#[test]
fn metrics_keep_call_count_separate_from_success_only_timing_samples() {
    let success = LongTermInvocationRow {
        id: 1,
        invoke_id: None,
        occurred_at: "2026-07-26T00:00:00Z".to_string(),
        status: Some("success".to_string()),
        model: Some("legacy-model".to_string()),
        request_model: Some("request-model".to_string()),
        response_model: Some("response-model".to_string()),
        reasoning_effort: Some("high".to_string()),
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: Some(100),
        output_tokens: Some(50),
        cost: Some(1.5),
        t_total_ms: Some(900.0),
        t_req_read_ms: Some(100.0),
        t_req_parse_ms: Some(50.0),
        t_upstream_connect_ms: Some(100.0),
        t_upstream_ttfb_ms: Some(300.0),
        t_upstream_stream_ms: Some(600.0),
        error_message: None,
    };
    let failure = LongTermInvocationRow {
        id: 2,
        invoke_id: None,
        occurred_at: "2026-07-26T00:00:01Z".to_string(),
        status: Some("failed".to_string()),
        model: None,
        request_model: None,
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: None,
        upstream_account_kind: None,
        upstream_account_name: None,
        total_tokens: None,
        output_tokens: Some(10),
        cost: None,
        t_total_ms: Some(100.0),
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: Some(20.0),
        t_upstream_stream_ms: Some(80.0),
        error_message: None,
    };
    let mut accumulator = LongTermAccumulator::default();
    accumulator.add_call(&success, Some((0, 900)));
    accumulator.add_call(&failure, None);
    let metrics = LongTermMetrics::from_accumulator(&accumulator);
    assert_eq!(metrics.calls, 2);
    assert_eq!(metrics.tokens, Some(100));
    assert_eq!(metrics.cost, Some(1.5));
    assert_eq!(metrics.usage_time_ms, Some(900.0));
    assert_eq!(metrics.output_speed_tokens_per_second, Some(50.0 / 0.6));
    assert_eq!(metrics.first_byte_ms, Some(550.0));
    assert_eq!(metrics.response_ms, Some(600.0));
    assert_eq!(normalize_long_term_model(&success), "response-model");
    assert_eq!(normalize_long_term_model(&failure), "未知模型");
    let mut blank_legacy = success.clone();
    blank_legacy.response_model = None;
    blank_legacy.model = Some("  \t".to_string());
    assert_eq!(normalize_long_term_model(&blank_legacy), "request-model");
}

#[test]
fn non_api_key_upstreams_share_the_other_series() {
    let row = LongTermInvocationRow {
        id: 1,
        invoke_id: None,
        occurred_at: "2026-07-26T00:00:00Z".to_string(),
        status: Some("success".to_string()),
        model: None,
        request_model: Some("gpt-5".to_string()),
        response_model: None,
        reasoning_effort: None,
        upstream_account_id: Some(7),
        upstream_account_kind: Some("oauth_codex".to_string()),
        upstream_account_name: Some("OAuth".to_string()),
        total_tokens: Some(1),
        output_tokens: Some(1),
        cost: Some(0.1),
        t_total_ms: None,
        t_req_read_ms: None,
        t_req_parse_ms: None,
        t_upstream_connect_ms: None,
        t_upstream_ttfb_ms: None,
        t_upstream_stream_ms: None,
        error_message: None,
    };
    assert_eq!(
        normalize_long_term_upstream(&row),
        ("other".to_string(), "其他".to_string())
    );
}

#[tokio::test]
async fn refresh_replaces_stale_rows_when_the_complete_live_source_is_empty() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(1);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    create_long_term_integrity_oracle(&pool).await;
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, total_tokens, cost, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms) VALUES (1, 'test-invoke-1', ?1, 'success', 'gpt-5', '{\"reasoningEffort\":\"high\"}', 8, 4, 0, 0, 12, 0.2, 100, 10, 5, 5, 20, 80)",
        )
        .bind(format!("{date}T12:00:00+08:00"))
        .execute(&pool)
        .await
        .expect("invocation row");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 1, 1, 12, 0.2, 12, 0.2)",
        )
        .bind(day_start + 12 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly proof");
    refresh_long_term_stats(&pool, 400).await.expect("refresh");
    let daily_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
    )
    .fetch_one(&pool)
    .await
    .expect("daily count");
    assert_eq!(daily_rows, 1);
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("state after initial refresh");
    assert_eq!(status, LONG_TERM_STATUS_READY);
    sqlx::query("DELETE FROM codex_invocations")
        .execute(&pool)
        .await
        .expect("remove live source row");
    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh after source removal");
    let remaining_daily_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE dimension = 'overall'",
    )
    .fetch_one(&pool)
    .await
    .expect("remaining daily count");
    assert_eq!(remaining_daily_rows, 0);
    let remaining_hourly_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_hourly WHERE dimension = 'overall'",
    )
    .fetch_one(&pool)
    .await
    .expect("remaining hourly count");
    assert_eq!(remaining_hourly_rows, 0);
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = 1")
            .fetch_one(&pool)
            .await
            .expect("state");
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_does_not_overwrite_complete_rollups_with_partial_rebuilds() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    sqlx::query(
        r#"
            CREATE TABLE codex_invocations (
                id INTEGER PRIMARY KEY,
                invoke_id TEXT,
                occurred_at TEXT NOT NULL,
                status TEXT,
                model TEXT,
                payload TEXT,
                total_tokens INTEGER,
                output_tokens INTEGER,
                cost REAL,
                t_total_ms REAL,
                t_req_read_ms REAL,
                t_req_parse_ms REAL,
                t_upstream_connect_ms REAL,
                t_upstream_ttfb_ms REAL,
                t_upstream_stream_ms REAL,
                error_message TEXT
            )
            "#,
    )
    .execute(&pool)
    .await
    .expect("invocation schema");
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    let occurred_at = format!("{today}T12:00:00+08:00");
    let day_start_epoch = today
        .and_hms_opt(0, 0, 0)
        .and_then(|value| Shanghai.from_local_datetime(&value).single())
        .expect("Shanghai start")
        .timestamp();
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, model, payload, total_tokens, output_tokens, cost) VALUES (1, 'partial', ?1, 'success', 'gpt-5', '{}', 7, 2, 0.07)",
        )
        .bind(occurred_at)
        .execute(&pool)
        .await
        .expect("partial source row");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 100, 10, 1.0, 10)",
        )
        .bind(today.to_string())
        .execute(&pool)
        .await
        .expect("complete daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 100, 10, 1.0, 10)",
        )
        .bind(day_start_epoch + 12 * 60 * 60)
        .execute(&pool)
        .await
        .expect("complete hourly rollup");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh protects complete rollups");

    let daily_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(today.to_string())
    .fetch_one(&pool)
    .await
    .expect("retained daily calls");
    let hourly_calls = sqlx::query_scalar::<_, i64>(
            "SELECT calls FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = 'overall'",
        )
        .bind(day_start_epoch + 12 * 60 * 60)
        .fetch_one(&pool)
        .await
        .expect("retained hourly calls");
    assert_eq!(daily_calls, 10);
    assert_eq!(hourly_calls, 10);
}

#[tokio::test]
async fn refresh_rebuilds_completed_rollups_after_complete_source_reconciliation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(1);
    let (day_start, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    let hour_start = day_start + 10 * 60 * 60;
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, terminal_proof_complete, total_tokens, total_cost) VALUES (?1, 'legacy', 2, 2, 200, 0.2, 0, 200, 0.2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("untrusted canonical rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(hour_start)
        .execute(&pool)
        .await
        .expect("durable hourly rollup");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh after complete source reconciliation");

    let daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("rebuilt daily rollup");
    let hourly = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_hourly WHERE bucket_start_epoch = ?1 AND dimension = 'overall'",
        )
        .bind(hour_start)
        .fetch_one(&pool)
        .await
        .expect("rebuilt hourly rollup");
    let queued_repairs =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("repair queue count");
    assert_eq!(daily, (1, 100, 0.1));
    assert_eq!(hourly, (1, 100, 0.1));
    assert_eq!(queued_repairs, 0);
}

#[tokio::test]
async fn initial_full_rebuild_publishes_a_complete_source_snapshot_without_hourly_proof() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("publish a complete initial source snapshot without canonical hourly proof");

    let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count materialized daily rollups");
    let queued_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("count deferred repairs");
    let state = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, statistics_start_date FROM long_term_stats_state WHERE id = ?1",
    )
    .bind(LONG_TERM_STATE_ID)
    .fetch_one(&pool)
    .await
    .expect("load initial full rebuild state");

    assert_eq!(materialized_rows, 1);
    assert_eq!(queued_repairs, 0);
    assert_eq!(state.0, LONG_TERM_STATUS_READY);
    let expected_start = date.to_string();
    assert_eq!(state.1.as_deref(), Some(expected_start.as_str()));
}

#[tokio::test]
async fn initial_full_rebuild_recovers_a_queued_repair_from_a_complete_source_snapshot() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'untrusted hourly proof')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("queue repair without canonical hourly proof");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("complete initial source snapshot resolves the queued repair");

    let materialized_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("count materialized daily rollups");
    let queued_repairs = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("count resolved repairs");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("load completed initial materialization state");

    assert_eq!(materialized_rows, 1);
    assert_eq!(queued_repairs, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_audits_and_repairs_historical_long_term_rollups_idempotently() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (first_hour, second_hour) = seed_long_term_integrity_case(&pool, date, 2).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("historical integrity repair");

    let daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("repaired daily rollup");
    assert_eq!(daily.0, 2);
    assert_eq!(daily.1, 200);
    assert!((daily.2 - 0.2).abs() < 1e-9);
    let hourly = sqlx::query_as::<_, (i64, i64)>(
            "SELECT bucket_start_epoch, calls FROM long_term_usage_hourly WHERE dimension = 'overall' AND bucket_start_epoch IN (?1, ?2) ORDER BY bucket_start_epoch",
        )
        .bind(first_hour)
        .bind(second_hour)
        .fetch_all(&pool)
        .await
        .expect("repaired hourly rollups");
    assert_eq!(hourly, vec![(first_hour, 1), (second_hour, 1)]);
    let model_calls = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'model'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("model dimension");
    let upstream_calls = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(calls), 0) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'upstream'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("upstream dimension");
    assert_eq!(model_calls, 2);
    assert_eq!(upstream_calls, 2);
    let queue_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("repair queue count");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("repair status");
    assert_eq!(queue_count, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("repeat historical integrity repair");
    let repeated_daily = sqlx::query_as::<_, (i64, i64, f64)>(
            "SELECT calls, token_total, cost_total FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("idempotent daily rollup");
    let repeated_hourly = sqlx::query_as::<_, (i64, i64)>(
            "SELECT bucket_start_epoch, calls FROM long_term_usage_hourly WHERE dimension = 'overall' AND bucket_start_epoch IN (?1, ?2) ORDER BY bucket_start_epoch",
        )
        .bind(first_hour)
        .bind(second_hour)
        .fetch_all(&pool)
        .await
        .expect("idempotent hourly rollups");
    assert_eq!(repeated_daily, daily);
    assert_eq!(repeated_hourly, hourly);
}

#[tokio::test]
async fn refresh_repairs_bad_rollups_after_complete_source_reconciliation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    seed_long_term_integrity_case(&pool, date, 1).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("complete source repair");

    let repaired_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repaired daily rollup");
    let queue_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repair queue count");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("ready state");
    assert_eq!(repaired_calls, 1);
    assert_eq!(queue_count, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_replaces_a_stale_day_with_empty_canonical_totals() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start, day_end) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("stale daily rollup");
    sqlx::query(
            "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 1, 100, 1, 0.1, 1)",
        )
        .bind(day_start + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("stale hourly rollup");
    for (dimension, series_key, display_name) in [
        ("model", "model:v2:stale", "stale model"),
        ("upstream", "upstream:stale", "stale upstream"),
    ] {
        sqlx::query(
                "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(date.to_string())
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension daily rollup");
        sqlx::query(
                "INSERT INTO long_term_usage_hourly (bucket_start_epoch, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, ?2, ?3, ?4, 1, 100, 1, 0.1, 1)",
            )
            .bind(day_start + 10 * 60 * 60)
            .bind(dimension)
            .bind(series_key)
            .bind(display_name)
            .execute(&pool)
            .await
            .expect("stale dimension hourly rollup");
    }
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("repair empty canonical day");

    let daily_rows = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("empty repaired daily rows");
    let hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
        )
        .bind(day_start)
        .bind(day_end)
        .fetch_one(&pool)
        .await
        .expect("empty repaired hourly rows");
    let queue_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("cleared repair queue");
    let status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("recovered status");
    assert_eq!(daily_rows, 0);
    assert_eq!(hourly_rows, 0);
    assert_eq!(queue_count, 0);
    assert_eq!(status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_applies_backoff_when_a_queued_repair_is_source_incomplete() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 2, 200, 2, 0.2, 2)",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("durable complete daily rollup");
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 2, 200, 0.2, 1, 100, 0.1, 'source data unavailable')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("due repair queue entry");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_READY)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("ready state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("incomplete repair is deferred");

    let first = sqlx::query_as::<_, (i64, i64, String)>(
            "SELECT attempts, expected_calls, next_retry_at FROM long_term_stats_repair_queue WHERE stats_date = ?1",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("backed-off repair queue entry");
    let durable_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("preserved daily rollup");
    assert_eq!(first.0, 1);
    assert_eq!(first.1, 2);
    assert!(!first.2.is_empty());
    assert_eq!(durable_calls, 2);

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("backoff suppresses immediate retry");
    let attempts = sqlx::query_scalar::<_, i64>(
        "SELECT attempts FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("persistent retry attempts");
    assert_eq!(attempts, 1);
}

#[tokio::test]
async fn refresh_replaces_a_queued_repair_with_an_empty_complete_source() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    create_long_term_test_invocations(&pool).await;
    create_long_term_integrity_oracle(&pool).await;
    let date = Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(3);
    let (day_start_epoch, _) = long_term_day_epoch_bounds(date).expect("Shanghai day bounds");
    sqlx::query(
            "INSERT INTO invocation_rollup_hourly (bucket_start_epoch, source, total_count, terminal_count, terminal_tokens, terminal_cost, total_tokens, total_cost) VALUES (?1, 'canonical', 1, 1, 100, 0.1, 100, 0.1)",
        )
        .bind(day_start_epoch + 10 * 60 * 60)
        .execute(&pool)
        .await
        .expect("canonical hourly rollup");
    sqlx::query(
            "INSERT INTO long_term_stats_repair_queue (stats_date, expected_calls, expected_token_total, expected_cost_total, observed_calls, observed_token_total, observed_cost_total, last_error) VALUES (?1, 1, 100, 0.1, 0, 0, 0, 'source data unavailable')",
        )
        .bind(date.to_string())
        .execute(&pool)
        .await
        .expect("queued repair");
    sqlx::query("UPDATE long_term_stats_state SET status = ?1 WHERE id = ?2")
        .bind(LONG_TERM_STATUS_ERROR)
        .bind(LONG_TERM_STATE_ID)
        .execute(&pool)
        .await
        .expect("error state");

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("empty complete source is reconciled");

    let first_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("pending error state");
    let queue_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM long_term_stats_repair_queue WHERE stats_date = ?1",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("queued repair count");
    let empty_daily_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
        )
        .bind(date.to_string())
        .fetch_one(&pool)
        .await
        .expect("empty daily row count");
    assert_eq!(first_status, LONG_TERM_STATUS_EMPTY);
    assert_eq!(queue_count, 0);
    assert_eq!(empty_daily_rows, 0);

    insert_long_term_test_invocation(&pool, 1, format!("{date}T10:00:00+08:00")).await;

    refresh_long_term_stats(&pool, 400)
        .await
        .expect("refresh materializes after source arrives");

    let repaired_calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(date.to_string())
    .fetch_one(&pool)
    .await
    .expect("repaired daily rollup");
    let queue_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM long_term_stats_repair_queue")
            .fetch_one(&pool)
            .await
            .expect("cleared repair queue");
    let final_status =
        sqlx::query_scalar::<_, String>("SELECT status FROM long_term_stats_state WHERE id = ?1")
            .bind(LONG_TERM_STATE_ID)
            .fetch_one(&pool)
            .await
            .expect("recovered ready state");
    assert_eq!(repaired_calls, 1);
    assert_eq!(queue_count, 0);
    assert_eq!(final_status, LONG_TERM_STATUS_READY);
}

#[tokio::test]
async fn refresh_recovers_after_a_real_sqlite_lock_releases() {
    let (pool, db_url, db_path) = long_term_file_backed_pool("long-term-lock-release").await;
    create_long_term_test_invocations(&pool).await;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    insert_long_term_test_invocation(&pool, 1, format!("{today}T12:00:00+08:00")).await;

    let mut lock_connection = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_connection)
        .await
        .expect("acquire sqlite write lock");
    let refresh_pool = pool.clone();
    let refresh_task =
        tokio::spawn(async move { refresh_long_term_stats(&refresh_pool, 400).await });

    sleep(Duration::from_millis(125)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_connection)
        .await
        .expect("release sqlite write lock");
    refresh_task
        .await
        .expect("join refresh task")
        .expect("refresh should retry after lock release");

    let calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(today.to_string())
    .fetch_one(&pool)
    .await
    .expect("published daily rollup");
    assert_eq!(calls, 1);

    lock_connection.close().await.expect("close lock holder");
    cleanup_long_term_file_backed_pool(pool, db_path).await;
}

#[tokio::test]
async fn refresh_exhausts_real_sqlite_locks_without_partial_publication() {
    let (pool, db_url, db_path) = long_term_file_backed_pool("long-term-lock-exhaustion").await;
    create_long_term_test_invocations(&pool).await;
    let today = Utc::now().with_timezone(&Shanghai).date_naive();
    insert_long_term_test_invocation(&pool, 1, format!("{today}T12:00:00+08:00")).await;
    sqlx::query(
            "INSERT INTO long_term_usage_daily (stats_date, dimension, series_key, display_name, calls, token_total, token_samples, cost_total, cost_samples) VALUES (?1, 'overall', 'overall', '全部调用', 10, 1000, 10, 1.0, 10)",
        )
        .bind(today.to_string())
        .execute(&pool)
        .await
        .expect("durable daily rollup");

    let mut lock_connection = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_connection)
        .await
        .expect("acquire sqlite write lock");
    let error = refresh_long_term_stats(&pool, 400)
        .await
        .expect_err("persistent sqlite lock should exhaust retry budget");
    assert!(crate::is_sqlite_lock_error(&error));

    sqlx::query("ROLLBACK")
        .execute(&mut lock_connection)
        .await
        .expect("release sqlite write lock");
    let calls = sqlx::query_scalar::<_, i64>(
        "SELECT calls FROM long_term_usage_daily WHERE stats_date = ?1 AND dimension = 'overall'",
    )
    .bind(today.to_string())
    .fetch_one(&pool)
    .await
    .expect("preserved daily rollup");
    let hourly_rows = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM long_term_usage_hourly WHERE bucket_start_epoch >= ?1 AND bucket_start_epoch < ?2",
        )
        .bind(
            long_term_day_epoch_bounds(today)
                .expect("Shanghai day bounds")
                .0,
        )
        .bind(
            long_term_day_epoch_bounds(today)
                .expect("Shanghai day bounds")
                .1,
        )
        .fetch_one(&pool)
        .await
        .expect("no partial hourly rollup");
    assert_eq!(calls, 10);
    assert_eq!(hourly_rows, 0);

    lock_connection.close().await.expect("close lock holder");
    cleanup_long_term_file_backed_pool(pool, db_path).await;
}

#[tokio::test]
async fn long_term_refresh_retries_a_sqlite_lock_until_the_write_is_available() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let outcome = run_long_term_refresh_with_retry_delays(
        None,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            async move {
                let attempt = operation_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(anyhow::anyhow!("database is locked"))
                } else {
                    Ok(attempt)
                }
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect("lock should clear before retry budget is exhausted");
    assert_eq!(outcome, 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn long_term_refresh_stops_after_its_bounded_sqlite_lock_retry_budget() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let error = run_long_term_refresh_with_retry_delays(
        None,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            async move {
                operation_attempts.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(anyhow::anyhow!("database is locked"))
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect_err("persistent locks must exhaust the bounded retry budget");
    assert!(crate::is_sqlite_lock_error(&error));
    assert_eq!(attempts.load(Ordering::SeqCst), 4);
}

#[tokio::test]
async fn long_term_refresh_cancels_before_a_second_sqlite_lock_retry() {
    let shutdown = CancellationToken::new();
    let operation_shutdown = shutdown.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let error = run_long_term_refresh_with_retry_delays(
        Some(&shutdown),
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            let operation_shutdown = operation_shutdown.clone();
            async move {
                operation_attempts.fetch_add(1, Ordering::SeqCst);
                operation_shutdown.cancel();
                Err::<(), _>(anyhow::anyhow!("database is locked"))
            }
        },
        &[Duration::from_secs(1)],
    )
    .await
    .expect_err("shutdown stops the initial refresh before another lock retry");
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn projection_flush_retries_a_transient_sqlite_lock() {
    let shutdown = CancellationToken::new();
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let outcome = run_long_term_projection_flush_with_retry_delays(
        &shutdown,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            async move {
                let attempt = operation_attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if attempt < 3 {
                    Err(anyhow::anyhow!("database is locked"))
                } else {
                    Ok(attempt)
                }
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect("transient lock retries before deferring a repair");
    assert_eq!(outcome, 3);
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn projection_flush_cancels_during_sqlite_lock_retry() {
    let shutdown = CancellationToken::new();
    let operation_shutdown = shutdown.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let operation_attempts = Arc::clone(&attempts);
    let error = run_long_term_projection_flush_with_retry_delays(
        &shutdown,
        move || {
            let operation_attempts = Arc::clone(&operation_attempts);
            let operation_shutdown = operation_shutdown.clone();
            async move {
                operation_attempts.fetch_add(1, Ordering::SeqCst);
                operation_shutdown.cancel();
                Err::<(), _>(anyhow::anyhow!("database is locked"))
            }
        },
        &[Duration::ZERO, Duration::ZERO, Duration::ZERO],
    )
    .await
    .expect_err("shutdown stops the lock retry without another write attempt");
    assert!(long_term_projection_write_is_deferred(&error));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn projection_date_rebuild_filters_rfc3339_rows_by_shanghai_epoch_bounds() {
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
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (1, 'utc-boundary', '2026-07-25T16:30:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("boundary invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (2, 'local-boundary', '2026-07-26 00:30:00', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("local boundary invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (3, 'rfc3339-crossing', '2026-07-25T15:59:59.500Z', 'success', 100, 600)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 crossing invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (4, 'legacy-crossing', '2026-07-25 23:59:59', 'success', 100, 2000)",
        )
        .execute(&pool)
        .await
        .expect("legacy crossing invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (5, 'rfc3339-outside', '2026-07-25T15:59:57Z', 'success', 100, 1000)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 outside invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (6, 'rfc3339-day-start', '2026-07-25T16:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 day-start invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (7, 'rfc3339-next-day-start', '2026-07-26T16:00:00Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 next-day-start invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (8, 'rfc3339-before-day-start', '2026-07-25T15:59:59.9999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-start invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (9, 'rfc3339-before-next-day-start', '2026-07-26T15:59:59.9999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-millisecond pre-end invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (10, 'rfc3339-submicro-before-day-start', '2026-07-25T15:59:59.9999999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-start invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (11, 'rfc3339-submicro-before-next-day-start', '2026-07-26T15:59:59.9999999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 sub-microsecond pre-end invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens, t_total_ms) VALUES (12, 'rfc3339-nanos-crossing', '2026-07-25T15:59:59.9999999Z', 'success', 100, 0.001)",
        )
        .execute(&pool)
        .await
        .expect("RFC3339 nanosecond crossing invocation");
    sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, total_tokens) VALUES (13, 'rfc3339-high-precision-before-start', '2026-07-25T15:59:59.99999999999999999999Z', 'success', 100)",
        )
        .execute(&pool)
        .await
        .expect("high-precision RFC3339 pre-start invocation");
    let date = NaiveDate::from_ymd_opt(2026, 7, 26).expect("fixed date");
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
    let crossing_query =
        long_term_projection_crossing_text_query("SELECT inv.id FROM codex_invocations inv");
    let plan = sqlx::query_as::<_, (i64, i64, i64, String)>(&format!(
        "EXPLAIN QUERY PLAN {crossing_query}"
    ))
    .bind(start.format("%Y-%m-%d %H:%M:%S").to_string())
    .fetch_all(&pool)
    .await
    .expect("crossing query plan");
    assert!(plan.iter().any(|(_, _, _, detail)| {
        detail.contains("idx_codex_invocations_long_term_projection_text_end")
    }));

    let control = LongTermProjectionWriteControl::unrestricted();
    let rows = load_long_term_projection_rows_for_date(&pool, date, start, end, &control)
        .await
        .expect("projection rows");

    assert_eq!(rows.len(), 8);
    let ids = rows
        .iter()
        .filter_map(|row| row.invoke_id.as_deref())
        .collect::<HashSet<_>>();
    assert_eq!(
        ids,
        HashSet::from([
            "utc-boundary",
            "local-boundary",
            "rfc3339-crossing",
            "legacy-crossing",
            "rfc3339-day-start",
            "rfc3339-before-next-day-start",
            "rfc3339-submicro-before-next-day-start",
            "rfc3339-nanos-crossing",
        ])
    );

    let rebuild = build_long_term_projection_date_rebuild(&pool, "2026-07-26", &control)
        .await
        .expect("projection rebuild retains the nanosecond crossing");
    assert!(
        rebuild
            .daily
            .keys()
            .any(|(bucket_date, dimension, series_key)| {
                bucket_date == "2026-07-26" && dimension == "overall" && series_key == "overall"
            }),
        "a selected positive crossing interval must materialize the target date"
    );
    assert!(rebuild.interval_segments.iter().any(|segment| {
        segment.invocation_row_id == 12
            && long_term_projection_interval_dates(segment).contains("2026-07-26")
    }));
}

#[tokio::test]
async fn projection_rebuild_preserves_a_newer_dirty_generation() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    ensure_long_term_stats_schema(&pool)
        .await
        .expect("long-term schema");
    let dates = vec!["2026-07-26".to_string()];
    queue_long_term_projection_repairs(&pool, &dates, "first")
        .await
        .expect("first repair");
    let stale_marker = load_long_term_projection_dirty_buckets(&pool, &dates)
        .await
        .expect("stale marker");
    queue_long_term_projection_repairs(&pool, &dates, "raced_correction")
        .await
        .expect("raced repair");

    commit_long_term_projection_date_rebuilds(&pool, &[], None, &stale_marker, false)
        .await
        .expect("commit stale rebuild");

    let generation = sqlx::query_scalar::<_, i64>(
        "SELECT generation FROM long_term_projection_dirty_buckets WHERE bucket_date = ?1",
    )
    .bind(&dates[0])
    .fetch_one(&pool)
    .await
    .expect("newer marker remains");
    assert_eq!(generation, 2);
}
