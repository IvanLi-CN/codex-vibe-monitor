fn build_summary_projection_archive_merge_live_record(
    now: chrono::DateTime<Utc>,
) -> SummaryProjectionRecord {
    SummaryProjectionRecord {
        row: UpstreamAccountInvocationPreviewRow {
            upstream_account_id: None,
            id: 7,
            invoke_id: "live-authoritative-copy".to_string(),
            prompt_cache_key: None,
            occurred_at: now.to_rfc3339(),
            conversation_created_at: None,
            status: "success".to_string(),
            live_phase: None,
            failure_class: None,
            route_mode: None,
            model: None,
            request_model: None,
            response_model: None,
            total_tokens: 17,
            cost: Some(1.7),
            cost_input: None,
            cost_cache_write: None,
            cost_cache_read: None,
            cost_output: None,
            cost_reasoning: None,
            source: Some(SOURCE_PROXY.to_string()),
            input_tokens: None,
            output_tokens: None,
            cache_input_tokens: None,
            reasoning_tokens: None,
            reasoning_effort: None,
            error_message: None,
            downstream_status_code: None,
            downstream_error_message: None,
            failure_kind: None,
            is_actionable: None,
            proxy_display_name: None,
            upstream_account_name: None,
            upstream_account_plan_type: None,
            response_content_encoding: None,
            request_compression_algorithm: None,
            transport: None,
            requested_service_tier: None,
            service_tier: None,
            billing_service_tier: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            first_token_ms: None,
            t_upstream_stream_ms: None,
            t_resp_parse_ms: None,
            t_persist_ms: None,
            t_total_ms: None,
            endpoint: None,
            compaction_request_kind: None,
            compaction_response_kind: None,
            image_intent: None,
        },
        occurred_at: now,
        global_rollup_covered: false,
        account_rollup_covered: false,
        usage_global_rollup_covered: false,
        usage_account_rollup_covered: false,
        is_persisted_live_record: true,
        is_archive_record: false,
        archive_has_materialized_rollups: false,
        account_archive_totals_fallback_included: false,
    }
}

#[tokio::test]
async fn summary_projection_archive_merge_prefers_resident_persisted_live_id() {
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory archive pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (\
             id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, \
             source TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, \
             cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, \
             cost_input REAL, cost_cache_write REAL, cost_cache_read REAL, cost_output REAL, \
             cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT, \
             failure_class TEXT)",
        )
        .execute(&archive_pool)
        .await
        .expect("create archive fixture table");
    sqlx::query(
            "INSERT INTO codex_invocations \
             (id, invoke_id, occurred_at, source, payload, total_tokens, cost, status) \
             VALUES (7, 'archive-replay-copy', datetime('now', '-1 minute'), 'proxy', '{}', 9, 1.0, 'success')",
        )
        .execute(&archive_pool)
        .await
        .expect("insert archive replay copy");

    let live_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory live pool");
    sqlx::query("CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY)")
        .execute(&live_pool)
        .await
        .expect("create live fixture table");
    sqlx::query("INSERT INTO codex_invocations (id) VALUES (7)")
        .execute(&live_pool)
        .await
        .expect("insert richer persisted live id");

    let now = Utc::now();
    let archive_range = ExactUtcRange {
        start: now - ChronoDuration::hours(2),
        end: now + ChronoDuration::minutes(1),
    };
    let mut records = HashMap::new();
    records.insert(
        "live-authoritative-copy".to_string(),
        build_summary_projection_archive_merge_live_record(now),
    );
    let mut budget = 0;
    let mut bytes = 0;
    merge_summary_projection_archive_records_with_coverage(SummaryProjectionArchiveMergeInput {
        archive_pool: &archive_pool,
        persisted_live_ids: &HashSet::new(),
        hourly_rollup_totals: &HashMap::new(),
        hourly_rollup_usage: &HashMap::new(),
        archive_has_materialized_rollups: false,
        overall_rollup_archive_replayed: None,
        account_rollup_archive_replayed: None,
        usage_rollup_archive_replayed: None,
        exact_range: archive_range,
        protected_boundary_buckets: &HashSet::new(),
        usage_rollup_cursor: None,
        known_account_ids: &HashSet::new(),
        records_by_invoke_id: &mut records,
        exact_record_budget: &mut budget,
        exact_record_bytes: &mut bytes,
        resident_current_record_bytes: 0,
        snapshot_rows: None,
    })
    .await
    .expect("bounded persisted-id lookup");
    assert_eq!(records.len(), 1);
    assert!(records.contains_key("live-authoritative-copy"));
}

async fn build_summary_projection_archive_global_replay_gap() -> (i64, SummaryProjectionRecord) {
    let archive_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory archive pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (\
             id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, \
             source TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER, \
             cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL, \
             cost_input REAL, cost_cache_write REAL, cost_cache_read REAL, cost_output REAL, \
             cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT, \
             failure_class TEXT)",
        )
        .execute(&archive_pool)
        .await
        .expect("create archive fixture table");
    let bucket = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0) - 3_600;
    let occurred_at = db_occurred_at_lower_bound(
        Utc.timestamp_opt(bucket + 300, 0)
            .single()
            .expect("valid archive row time"),
    );
    sqlx::query(
        "INSERT INTO codex_invocations \
             (id, invoke_id, occurred_at, source, payload, total_tokens, cost, status) \
             VALUES (8, 'archive-global-replay-gap', ?1, 'proxy', '{}', 9, 1.0, 'success')",
    )
    .bind(occurred_at)
    .execute(&archive_pool)
    .await
    .expect("insert archive replay row");
    let _live_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory live pool");
    let archive_range = ExactUtcRange {
        start: Utc
            .timestamp_opt(bucket, 0)
            .single()
            .expect("valid range start"),
        end: Utc
            .timestamp_opt(bucket + 3_600, 0)
            .single()
            .expect("valid range end"),
    };
    let mut totals = HashMap::new();
    totals.insert((bucket, None), StatsTotals::default());
    let mut usage = HashMap::new();
    usage.insert(
        (bucket, None),
        UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
            models: Vec::new(),
        },
    );
    let mut records = HashMap::new();
    let mut budget = 0;
    let mut bytes = 0;
    merge_summary_projection_archive_records_with_coverage(SummaryProjectionArchiveMergeInput {
        archive_pool: &archive_pool,
        persisted_live_ids: &HashSet::new(),
        hourly_rollup_totals: &totals,
        hourly_rollup_usage: &usage,
        archive_has_materialized_rollups: true,
        overall_rollup_archive_replayed: Some(false),
        account_rollup_archive_replayed: Some(true),
        usage_rollup_archive_replayed: Some(true),
        exact_range: archive_range,
        protected_boundary_buckets: &HashSet::new(),
        usage_rollup_cursor: None,
        known_account_ids: &HashSet::new(),
        records_by_invoke_id: &mut records,
        exact_record_budget: &mut budget,
        exact_record_bytes: &mut bytes,
        resident_current_record_bytes: 0,
        snapshot_rows: None,
    })
    .await
    .expect("merge exact archive row when overall replay is missing");
    let record = records
        .remove("archive-global-replay-gap")
        .expect("missing overall replay keeps the exact archive row");
    (bucket, record)
}

#[tokio::test]
async fn summary_projection_archive_merge_keeps_exact_global_row_without_overall_replay() {
    let (bucket, record) = build_summary_projection_archive_global_replay_gap().await;
    assert!(!record.global_rollup_covered);
    assert_eq!(record.row.total_tokens, 9);
    assert!(
        summary_projection_all_time_uses_global_exact_record(&record, 100),
        "a materialized raw fallback contributes when its global compact bucket is absent"
    );
    let mut account_record = record.clone();
    account_record.row.upstream_account_id = Some(42);
    account_record.account_rollup_covered = false;
    assert!(
        summary_projection_all_time_uses_account_exact_record(&account_record, Some(100)),
        "a missing account compact bucket retains the exact materialized source"
    );
    account_record.account_rollup_covered = true;
    assert!(
        !summary_projection_all_time_uses_account_exact_record(&account_record, Some(100)),
        "a complete account compact bucket must not double count the raw fallback"
    );

    // Replacing a partial compact aggregate requires every co-located live contribution,
    // not only the archive repair row. This models a retained terminal failure whose source
    // archive prunes successful detail rows after materialization.
    let mut retained_live_record = record.clone();
    retained_live_record.row.id = 9;
    retained_live_record.row.invoke_id = "retained-live-tail".to_string();
    retained_live_record.row.total_tokens = 3;
    retained_live_record.row.cost = Some(0.2);
    retained_live_record.row.status = "failed".to_string();
    retained_live_record.row.failure_class = Some("service_failure".to_string());
    retained_live_record.is_archive_record = false;
    retained_live_record.is_persisted_live_record = true;
    retained_live_record.global_rollup_covered = false;

    // The exact bucket replaces the partial compact aggregate, then adds both the archive
    // and retained live records from the canonical projection.
    let projection = SummaryProjection {
        records: vec![record.clone(), retained_live_record],
        hourly_buckets: BTreeMap::from([(bucket, vec![0, 1])]),
        hourly_rollup_totals: HashMap::from([(
            (bucket, None),
            StatsTotals {
                total_count: 1,
                success_count: 1,
                total_tokens: 4,
                total_cost: 0.5,
                ..StatsTotals::default()
            },
        )]),
        exact_global_total_rollup_buckets: HashSet::from([bucket]),
        refreshed_at: Some(Instant::now()),
        ..SummaryProjection::default()
    };
    let response = projection
        .response_for_query(
            &SummaryQuery {
                window: Some("2h".to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            },
            100,
        )
        .expect("fresh in-memory range response");
    assert_eq!(response.total_count, 2);
    assert_eq!(response.success_count, 1);
    assert_eq!(response.failure_count, 1);
    assert_eq!(response.total_tokens, 12);
    assert_eq!(response.total_cost, 1.2);
    assert_eq!(response.non_success_cost, Some(0.2));
}

#[tokio::test]
async fn summary_projection_fails_closed_when_unmaterialized_archive_exceeds_budget() {
    const TEST_EXACT_RECORD_LIMIT: usize = 8;

    with_summary_projection_test_exact_record_limit(TEST_EXACT_RECORD_LIMIT, async {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory archive pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (\
             id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL,\
             source TEXT, model TEXT, payload TEXT, input_tokens INTEGER, output_tokens INTEGER,\
             cache_input_tokens INTEGER, reasoning_tokens INTEGER, total_tokens INTEGER, cost REAL,\
             cost_input REAL, cost_cache_write REAL, cost_cache_read REAL, cost_output REAL,\
             cost_reasoning REAL, status TEXT, error_message TEXT, failure_kind TEXT,\
             failure_class TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create archive fixture table");
        sqlx::query(
            "WITH RECURSIVE rows(value) AS (SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < ?1) \
             INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, model, payload, \
             input_tokens, output_tokens, total_tokens, cost, status) \
             SELECT value, 'archive-summary-' || value, datetime('now', '-1 minute'), 'proxy', \
             'gpt-5', '{}', 1, 1, 2, 0.1, 'success' FROM rows",
        )
        .bind((TEST_EXACT_RECORD_LIMIT + 1) as i64)
        .execute(&pool)
        .await
        .expect("insert unmaterialized archive fixture");

        // SQLite's `datetime('now')` is UTC while persisted naive timestamps are interpreted as
        // Asia/Shanghai.  Use the matching UTC range so this fixture proves the LIMIT/overflow
        // path instead of silently selecting an empty range.
        let now = Utc::now();
        let archive_range = ExactUtcRange {
            start: now - ChronoDuration::hours(9),
            end: now - ChronoDuration::hours(7),
        };
        let mut records = HashMap::new();
        let mut budget = 0;
        let error = merge_summary_projection_archive_records(SummaryProjectionArchiveRecordsInput {
            pool: &pool,
            persisted_live_ids: &HashSet::new(),
            hourly_rollup_totals: &HashMap::new(),
            hourly_rollup_usage: &HashMap::new(),
            archive_has_materialized_rollups: false,
            exact_range: archive_range,
            protected_boundary_buckets: &HashSet::new(),
            usage_rollup_cursor: None,
            known_account_ids: &HashSet::new(),
            records_by_invoke_id: &mut records,
            exact_record_budget: &mut budget,
        })
        .await
        .expect_err("unmaterialized archive overflow must fail closed");
        assert!(error.to_string().contains("exact-record budget"));
        assert!(records.len() <= TEST_EXACT_RECORD_LIMIT);

        let mut materialized_records = HashMap::new();
        let mut materialized_budget = 0;
        merge_summary_projection_archive_records(SummaryProjectionArchiveRecordsInput {
            pool: &pool,
            persisted_live_ids: &HashSet::new(),
            hourly_rollup_totals: &HashMap::from([(
                (
                    align_bucket_epoch((now - ChronoDuration::hours(8)).timestamp(), 3_600, 0),
                    None,
                ),
                StatsTotals::default(),
            )]),
            hourly_rollup_usage: &HashMap::from([(
                (
                    align_bucket_epoch((now - ChronoDuration::hours(8)).timestamp(), 3_600, 0),
                    None,
                ),
                UsageBreakdownResponse {
                    cache_write_tokens: 0,
                    cache_read_tokens: 0,
                    output_tokens: 0,
                    costs: None,
                    models: Vec::new(),
                },
            )]),
            archive_has_materialized_rollups: true,
            exact_range: archive_range,
            protected_boundary_buckets: &HashSet::new(),
            usage_rollup_cursor: None,
            known_account_ids: &HashSet::new(),
            records_by_invoke_id: &mut materialized_records,
            exact_record_budget: &mut materialized_budget,
        })
        .await
        .expect("materialized archive outside protected boundaries must stay bounded");
        assert!(materialized_records.is_empty());
        })
        .await;
}

#[tokio::test]
async fn summary_projection_bounds_current_index_and_serves_recent_limit_from_it() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let fixture_now = Utc::now();
    for index in 0..80_i64 {
        let occurred_at =
            db_occurred_at_lower_bound(fixture_now - ChronoDuration::seconds(index + 2));
        sqlx::query(
                "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
                 VALUES (?1, ?2, 'proxy', 'success', ?3, 1.0, '{\"upstreamAccountId\":42}', '', 'full')",
            )
            .bind(format!("summary-current-{index}"))
            .bind(occurred_at)
            .bind(index + 1)
            .execute(&state.pool)
            .await
            .expect("insert current fixture row");
    }
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate bounded recent index");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("hydrated projection");
    let max = state.config.list_limit_max as usize;
    assert!(projection.recent_indexes[&None].len() <= max);
    assert!(projection.recent_indexes[&Some(42)].len() <= max);
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(3),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: Some(42),
        }),
    )
    .await
    .expect("serve bounded current index from memory");
    assert_eq!(response.total_count, 3);
    assert_eq!(response.total_tokens, 3 + 2 + 1);
}
