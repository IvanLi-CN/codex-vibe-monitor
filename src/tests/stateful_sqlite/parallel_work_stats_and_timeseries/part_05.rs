struct SummaryArchivePartsSpec<'a> {
    first_batch: &'a str,
    first_invoke_id: &'a str,
    first_id: i64,
    second_batch: &'a str,
    second_invoke_id: &'a str,
    second_id: i64,
}

struct SummaryArchiveParts {
    first_path: PathBuf,
    first_at: String,
}

enum FirstArchiveRollupState {
    RollupOnly,
    InvocationReplayed,
    FullyReplayedUnreadable,
}

async fn seed_summary_archive_parts(
    state: &Arc<AppState>,
    spec: SummaryArchivePartsSpec<'_>,
) -> SummaryArchiveParts {
    let hour = (Utc::now().with_timezone(&Shanghai).date_naive() - ChronoDuration::days(10))
        .and_hms_opt(7, 0, 0)
        .expect("valid summary archive-parts hour");
    let first_at = format_naive(
        hour.checked_add_signed(ChronoDuration::minutes(5))
            .expect("first summary archive-parts time"),
    );
    let second_at = format_naive(
        hour.checked_add_signed(ChronoDuration::minutes(25))
            .expect("second summary archive-parts time"),
    );
    let original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        spec.first_batch,
        &[(
            spec.first_id,
            spec.first_invoke_id,
            &first_at,
            SOURCE_PROXY,
            "success",
            10,
            0.10,
            Some(100.0),
        )],
    )
    .await;
    let first_path = state
        .config
        .archive_dir
        .join(format!("{}.sqlite.gz", spec.first_batch));
    let _ = fs::remove_file(&first_path);
    fs::rename(&original_path, &first_path).expect("move first summary archive part");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1 WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(first_path.to_string_lossy().to_string())
    .bind(original_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("update first summary archive-part path");
    seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        spec.second_batch,
        &[(
            spec.second_id,
            spec.second_invoke_id,
            &second_at,
            SOURCE_PROXY,
            "success",
            20,
            0.20,
            Some(120.0),
        )],
    )
    .await;
    SummaryArchiveParts {
        first_path,
        first_at,
    }
}

async fn materialize_first_summary_archive_part(
    state: &Arc<AppState>,
    fixture: &SummaryArchiveParts,
    rollup_state: FirstArchiveRollupState,
) {
    let bucket = invocation_bucket_start_epoch(&fixture.first_at)
        .expect("summary archive-part bucket start");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
            first_byte_max_ms, first_byte_histogram
        ) VALUES (?1, ?2, 1, 1, 0, 10, 0.10, 1, 100, 100, ?3)
        "#,
    )
    .bind(bucket)
    .bind(SOURCE_PROXY)
    .bind("[0,0,0,0,0,0,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0]")
    .execute(&state.pool)
    .await
    .expect("seed first summary archive-part rollup");
    if matches!(rollup_state, FirstArchiveRollupState::RollupOnly) {
        return;
    }
    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?1",
    )
    .bind(fixture.first_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark first summary archive part materialized");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket,
        SOURCE_PROXY,
    )
    .await;
    match rollup_state {
        FirstArchiveRollupState::RollupOnly => unreachable!(),
        FirstArchiveRollupState::InvocationReplayed => {
            insert_hourly_rollup_archive_replay_marker(
                &state.pool,
                HOURLY_ROLLUP_TARGET_INVOCATIONS,
                &fixture.first_path,
            )
            .await;
        }
        FirstArchiveRollupState::FullyReplayedUnreadable => {
            mark_summary_archive_replay_complete(&state.pool, &fixture.first_path).await;
            fs::write(&fixture.first_path, b"not-a-gzip-archive")
                .expect("corrupt first summary archive part");
        }
    }
}

pub(crate) fn assert_two_success_summary(summary: &StatsResponse) {
    assert_eq!(summary.total_count, 2);
    assert_eq!(summary.success_count, 2);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 30);
    assert_f64_close(summary.total_cost, 0.30);
}

async fn seed_clipped_boundary_rollups(
    state: &Arc<AppState>,
    archive_path: &Path,
    before_boundary: &str,
    after_boundary: &str,
) {
    let mut materialized_buckets = std::collections::HashSet::new();
    for (occurred_at, total_tokens, total_cost) in [
        (before_boundary, 10_i64, 0.10_f64),
        (after_boundary, 20_i64, 0.20_f64),
    ] {
        let bucket = invocation_bucket_start_epoch(occurred_at)
            .expect("clipped-boundary hour should have a durable bucket");
        sqlx::query(
            "INSERT INTO invocation_rollup_hourly \
             (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost) \
             VALUES (?1, ?2, 1, 1, 0, ?3, ?4) \
             ON CONFLICT(bucket_start_epoch, source) DO UPDATE SET \
                 total_count = total_count + excluded.total_count, \
                 success_count = success_count + excluded.success_count, \
                 total_tokens = total_tokens + excluded.total_tokens, \
                 total_cost = total_cost + excluded.total_cost",
        )
        .bind(bucket)
        .bind(SOURCE_PROXY)
        .bind(total_tokens)
        .bind(total_cost)
        .execute(&state.pool)
        .await
        .expect("seed full-hour durable rollup");
        if materialized_buckets.insert(bucket) {
            insert_materialized_rollup_bucket_marker(
                &state.pool,
                HOURLY_ROLLUP_TARGET_INVOCATIONS,
                bucket,
                SOURCE_PROXY,
            )
            .await;
        }
    }
    mark_summary_archive_replay_complete(&state.pool, archive_path).await;
    fs::write(archive_path, b"not-a-gzip-archive")
        .expect("corrupt clipped-boundary archive after durable replay");
}

#[tokio::test]
pub(crate) async fn all_time_summary_fallback_aggregates_missing_rows_across_archive_parts() {
    let state = archive_retention_test_state().await;
    let fixture = seed_summary_archive_parts(
        &state,
        SummaryArchivePartsSpec {
            first_batch: "summary-multipart-archive-a",
            first_invoke_id: "summary-multipart-first",
            first_id: 101,
            second_batch: "summary-multipart-archive-b",
            second_invoke_id: "summary-multipart-second",
            second_id: 201,
        },
    )
    .await;
    materialize_first_summary_archive_part(&state, &fixture, FirstArchiveRollupState::RollupOnly)
        .await;
    let summary = fetch_test_summary(state, "all").await;
    assert_two_success_summary(&summary);
}

#[tokio::test]
pub(crate) async fn all_time_summary_fallback_keeps_unmaterialized_rows_when_sibling_archive_part_is_materialized()
 {
    let state = archive_retention_test_state().await;
    let fixture = seed_summary_archive_parts(
        &state,
        SummaryArchivePartsSpec {
            first_batch: "summary-mixed-state-archive-a",
            first_invoke_id: "summary-mixed-state-first",
            first_id: 1,
            second_batch: "summary-mixed-state-archive-b",
            second_invoke_id: "summary-mixed-state-second",
            second_id: 1,
        },
    )
    .await;
    materialize_first_summary_archive_part(
        &state,
        &fixture,
        FirstArchiveRollupState::InvocationReplayed,
    )
    .await;
    let summary = fetch_test_summary(state, "all").await;
    assert_two_success_summary(&summary);
}

#[tokio::test]
pub(crate) async fn all_time_summary_fallback_keeps_unmaterialized_rows_when_materialized_sibling_archive_is_unreadable()
 {
    let state = archive_retention_test_state().await;
    let fixture = seed_summary_archive_parts(
        &state,
        SummaryArchivePartsSpec {
            first_batch: "summary-mixed-state-unreadable-archive-a",
            first_invoke_id: "summary-mixed-state-unreadable-first",
            first_id: 1,
            second_batch: "summary-mixed-state-unreadable-archive-b",
            second_invoke_id: "summary-mixed-state-unreadable-second",
            second_id: 1,
        },
    )
    .await;
    materialize_first_summary_archive_part(
        &state,
        &fixture,
        FirstArchiveRollupState::FullyReplayedUnreadable,
    )
    .await;
    let summary = fetch_test_summary(state, "all").await;
    assert_two_success_summary(&summary);
}

#[tokio::test]
pub(crate) async fn summary_projection_rejects_clipped_rolling_boundary_for_unreadable_replayed_archive()
 {
    let state = archive_retention_test_state().await;

    let boundary = Utc::now() - ChronoDuration::days(7);
    let before_boundary = format_naive(
        (boundary - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let after_boundary = format_naive(
        (boundary + ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-clipped-boundary-unreadable",
        &[
            (
                1_i64,
                "summary-clipped-boundary-before",
                before_boundary.as_str(),
                SOURCE_PROXY,
                "success",
                10_i64,
                0.10_f64,
                Some(100.0),
            ),
            (
                2_i64,
                "summary-clipped-boundary-after",
                after_boundary.as_str(),
                SOURCE_PROXY,
                "success",
                20_i64,
                0.20_f64,
                Some(120.0),
            ),
        ],
    )
    .await;
    let archive_path = state
        .config
        .archive_dir
        .join("summary-clipped-boundary-unreadable.sqlite.gz");
    let _ = fs::remove_file(&archive_path);
    fs::rename(&original_path, &archive_path)
        .expect("move clipped-boundary archive to a unique path");
    sqlx::query(
        "UPDATE archive_batches \
         SET file_path = ?1, historical_rollups_materialized_at = datetime('now'), \
             coverage_start_at = ?3, coverage_end_at = ?4 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(original_path.to_string_lossy().to_string())
    .bind(&before_boundary)
    .bind(&after_boundary)
    .execute(&state.pool)
    .await
    .expect("mark clipped-boundary archive as materialized");

    seed_clipped_boundary_rollups(&state, &archive_path, &before_boundary, &after_boundary).await;

    let all_time = fetch_test_summary_in_zone(state.clone(), "all", "UTC").await;
    assert_eq!(all_time.total_count, 2);
    assert_eq!(all_time.total_tokens, 30);

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("a materialized unreadable boundary must publish the rollup-backed projection");
    state.pool.close().await;
    let rolling = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("7d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(rolling, Err(ApiError::Unavailable(_))),
        "a clipped rolling boundary must remain unavailable without request-time I/O: {rolling:?}"
    );
}

#[tokio::test]
pub(crate) async fn summary_projection_replaces_rollup_for_point_materialized_archive_without_replay()
 {
    let state = archive_retention_test_state().await;
    let archived_at_utc = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::hours(6)).timestamp(),
                3_600,
                0,
            ) + 15 * 60,
            0,
        )
        .single()
        .expect("align point archive fixture");
    let archived_at = format_naive(archived_at_utc.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-point-materialized-without-replay",
        &[(
            9_001,
            "summary-point-materialized-without-replay",
            archived_at.as_str(),
            SOURCE_PROXY,
            "success",
            17,
            1.25,
            Some(120.0),
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches \
         SET coverage_start_at = ?1, coverage_end_at = ?1, historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(&archived_at)
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark point archive materialized without replay");
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
    )
    .bind(crate::stats::align_bucket_epoch(
        archived_at_utc.timestamp(),
        3_600,
        0,
    ))
    .execute(&state.pool)
    .await
    .expect("seed compact point rollup");

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate point materialized archive replacement");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve point archive range without SQLite or archive I/O");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.success_count, 1);
    assert_eq!(response.total_tokens, 17);
    assert_f64_close(response.total_cost, 1.25);
}

#[tokio::test]
pub(crate) async fn summary_projection_keeps_rollup_ranges_when_materialized_current_archive_exceeds_admission()
 {
    with_summary_projection_test_exact_record_limit(10_000, async {
        summary_projection_keeps_rollup_ranges_when_materialized_current_archive_exceeds_admission_with_limit().await;
    })
    .await;
}

async fn seed_over_admission_archive(state: &Arc<AppState>) -> (DateTime<Utc>, PathBuf) {
    let archive_hour = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (Utc::now() - ChronoDuration::hours(6)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align current archive fixture");
    let archive_db_path = state
        .config
        .archive_dir
        .join("summary-current-materialized-over-admission.sqlite");
    let archive_path = state
        .config
        .archive_dir
        .join("summary-current-materialized-over-admission.sqlite.gz");
    fs::create_dir_all(&state.config.archive_dir).expect("create current archive directory");
    fs::File::create(&archive_db_path).expect("create current archive SQLite source");
    let archive_pool = SqlitePool::connect(&test_sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open current archive SQLite source");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create current archive schema");
    let archive_start = format_naive(archive_hour.with_timezone(&Shanghai).naive_local());
    let archive_last = format_naive(
        (archive_hour + ChronoDuration::hours(1) - ChronoDuration::seconds(1))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let archive_middle = format_naive(
        (archive_hour + ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    sqlx::query(
        "WITH RECURSIVE rows(id) AS (SELECT 1 UNION ALL SELECT id + 1 FROM rows WHERE id < 10001) \
         INSERT INTO codex_invocations \
         (id, invoke_id, occurred_at, source, status, total_tokens, cost, detail_level, payload, raw_response, created_at) \
         SELECT id, 'summary-current-materialized-over-admission-' || id, \
                CASE WHEN id = 1 THEN ?1 WHEN id = 10001 THEN ?2 ELSE ?3 END, \
                'proxy', 'success', 1, 0.01, 'full', '{}', '', ?3 FROM rows",
    )
    .bind(&archive_start)
    .bind(&archive_last)
    .bind(&archive_middle)
    .execute(&archive_pool)
    .await
    .expect("seed current archive over bounded candidate admission");
    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_path)
        .expect("compress current archive source");
    let archive_sha256 = sha256_hex_file(&archive_path).expect("hash current archive source");
    fs::remove_file(&archive_db_path).expect("remove current archive SQLite source");
    sqlx::query(
        "INSERT INTO archive_batches \
         (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
          historical_rollups_materialized_at, created_at) \
         VALUES ('codex_invocations', ?1, ?2, ?3, 10001, 'completed', ?4, ?5, datetime('now'), datetime('now'))",
    )
    .bind(archive_start[..7].to_string())
    .bind(archive_path.to_string_lossy().to_string())
    .bind(archive_sha256)
    .bind(archive_start)
    .bind(archive_last)
    .execute(&state.pool)
    .await
    .expect("insert materialized current archive manifest");
    (archive_hour, archive_path)
}

async fn seed_over_admission_rollups(
    state: &Arc<AppState>,
    archive_hour: DateTime<Utc>,
    archive_path: &Path,
) {
    sqlx::query(
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 10001, 10001, 0, 10001, 100.01, 0)",
    )
    .bind(archive_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed complete materialized current summary rollup");
    sqlx::query(
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
          request_count, output_tokens, cost_unknown, has_cost) \
         VALUES (?1, 'proxy', '-1', '', '', 10001, 10001, 100.01, 1)",
    )
    .bind(archive_hour.timestamp())
    .execute(&state.pool)
    .await
    .expect("seed complete materialized current usage rollup");
    mark_summary_archive_replay_complete(&state.pool, archive_path).await;
}

pub(crate) async fn summary_projection_keeps_rollup_ranges_when_materialized_current_archive_exceeds_admission_with_limit()
 {
    let state = archive_retention_test_state().await;
    let (archive_hour, archive_path) = seed_over_admission_archive(&state).await;
    seed_over_admission_rollups(&state, archive_hour, &archive_path).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("current candidate admission must not abort rollup-backed projection hydration");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("reconcile the exact all-time rollup-backed projection");
    state.pool.close().await;

    for window in ["1d", "all"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: None,
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("serve {window} from the rollup-backed memory projection");
        assert_eq!(response.total_count, 10_001, "{window} count");
        assert_eq!(response.total_tokens, 10_001, "{window} tokens");
        assert_f64_close(response.total_cost, 100.01);
    }
    assert!(matches!(
        fetch_summary(
            State(state),
            Query(SummaryQuery {
                window: Some("current".to_string()),
                limit: Some(1),
                time_zone: Some("UTC".to_string()),
                upstream_account_id: None,
            }),
        )
        .await,
        Err(ApiError::Unavailable(_))
    ));
}

async fn seed_compact_current_archive(state: &Arc<AppState>, now: DateTime<Utc>) {
    let live_at = db_occurred_at_lower_bound(now - ChronoDuration::minutes(1));
    sqlx::query(
        "WITH RECURSIVE rows(value) AS ( \
            SELECT 1 UNION ALL SELECT value + 1 FROM rows WHERE value < 50000 \
         ) INSERT INTO codex_invocations \
         (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
         SELECT 'summary-compact-current-live-' || value, ?1, 'proxy', 'success', 1, 0.1, '{}', '', 'full' \
         FROM rows",
    )
    .bind(live_at)
    .execute(&state.pool)
    .await
    .expect("seed bounded current resident prefix");
    let archive_hour = Utc
        .timestamp_opt(
            crate::stats::align_bucket_epoch(
                (now - ChronoDuration::hours(3)).timestamp(),
                3_600,
                0,
            ),
            0,
        )
        .single()
        .expect("align compact archive hour");
    let archive_at = archive_hour + ChronoDuration::minutes(15);
    let occurred_at = format_naive(archive_at.with_timezone(&Shanghai).naive_local());
    let archive_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        "summary-compact-current-prune",
        &[(
            60_001,
            "summary-compact-current-pruned",
            &occurred_at,
            SOURCE_PROXY,
            "success",
            17,
            1.25,
            None,
        )],
    )
    .await;
    sqlx::query(
        "UPDATE archive_batches SET coverage_start_at = ?1, coverage_end_at = ?2, \
         historical_rollups_materialized_at = datetime('now') \
         WHERE dataset = 'codex_invocations' AND file_path = ?3",
    )
    .bind(db_occurred_at_lower_bound(archive_at))
    .bind(db_occurred_at_lower_bound(
        archive_at + ChronoDuration::seconds(1),
    ))
    .bind(archive_path.to_string_lossy().to_string())
    .execute(&state.pool)
    .await
    .expect("mark exact compact archive coverage");
    for query in [
        "INSERT INTO invocation_rollup_hourly \
         (bucket_start_epoch, source, total_count, success_count, failure_count, total_tokens, total_cost, non_success_cost) \
         VALUES (?1, 'proxy', 1, 1, 0, 17, 1.25, 0)",
        "INSERT INTO upstream_account_usage_breakdown_hourly \
         (bucket_start_epoch, source, upstream_account_key, normalized_model, normalized_reasoning_effort, \
          request_count, output_tokens, cost_unknown, has_cost) \
         VALUES (?1, 'proxy', '-1', '', '', 1, 17, 1.25, 1)",
    ] {
        sqlx::query(query)
            .bind(archive_hour.timestamp())
            .execute(&state.pool)
            .await
            .expect("seed compact archive rollup");
    }
    mark_summary_archive_replay_complete(&state.pool, &archive_path).await;
}

#[tokio::test]
pub(crate) async fn summary_projection_pruned_compact_current_candidate_keeps_rolling_range_exact()
{
    let state =
        test_state_with_openai_base(Url::parse("https://api.openai.com/").expect("valid test URL"))
            .await;
    let now = Utc::now();
    seed_compact_current_archive(&state, now).await;

    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate the compact-proof current candidate");
    state.pool.close().await;

    let Json(rolling) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("serve compact-rollup-proven rolling range without SQLite");
    assert_eq!(rolling.total_count, 50_001);
    assert_eq!(rolling.total_tokens, 50_017);
    assert_f64_close(rolling.total_cost, 5_001.25);

    let Json(current) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(1),
            time_zone: Some("UTC".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("current cutoff remains exact after compact candidate pruning");
    assert_eq!(current.total_count, 1);
}

pub(crate) async fn seed_materialized_summary_archive(
    state: &Arc<AppState>,
    batch_name: &str,
    occurred_at: &str,
    total_tokens: i64,
    total_cost: f64,
    ttfb_ms: f64,
) -> PathBuf {
    let original_path = seed_invocation_archive_batch(
        &state.pool,
        &state.config,
        batch_name,
        &[(
            1,
            batch_name,
            occurred_at,
            SOURCE_PROXY,
            "success",
            total_tokens,
            total_cost,
            Some(ttfb_ms),
        )],
    )
    .await;
    let archive_path = state
        .config
        .archive_dir
        .join(format!("{batch_name}.sqlite.gz"));
    let _ = fs::remove_file(&archive_path);
    fs::rename(&original_path, &archive_path).expect("move materialized summary archive");
    sqlx::query(
        "UPDATE archive_batches SET file_path = ?1, \
         historical_rollups_materialized_at = datetime('now'), \
         coverage_start_at = ?3, coverage_end_at = ?3 \
         WHERE dataset = 'codex_invocations' AND file_path = ?2",
    )
    .bind(archive_path.to_string_lossy().to_string())
    .bind(original_path.to_string_lossy().to_string())
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("mark summary archive materialized");
    archive_path
}

async fn seed_same_bucket_summary_rollup(
    state: &Arc<AppState>,
    occurred_at: &str,
    archive_paths: [&Path; 2],
) {
    let bucket = invocation_bucket_start_epoch(occurred_at).expect("same-bucket summary bucket");
    let mut histogram = empty_approx_histogram();
    add_approx_histogram_sample(&mut histogram, 100.0);
    add_approx_histogram_sample(&mut histogram, 120.0);
    let histogram = encode_approx_histogram(&histogram).expect("encode summary histogram");
    sqlx::query(
        r#"
        INSERT INTO invocation_rollup_hourly (
            bucket_start_epoch, source, total_count, success_count, failure_count,
            total_tokens, total_cost, first_byte_sample_count, first_byte_sum_ms,
            first_byte_max_ms, first_byte_histogram
        ) VALUES (?1, ?2, 2, 2, 0, 30, 0.30, 2, 220, 120, ?3)
        "#,
    )
    .bind(bucket)
    .bind(SOURCE_PROXY)
    .bind(histogram)
    .execute(&state.pool)
    .await
    .expect("seed same-bucket summary rollup row");
    insert_materialized_rollup_bucket_marker(
        &state.pool,
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        bucket,
        SOURCE_PROXY,
    )
    .await;
    for archive_path in archive_paths {
        mark_summary_archive_replay_complete(&state.pool, archive_path).await;
    }
}

async fn assert_same_bucket_summary_timeseries(state: Arc<AppState>, hour: NaiveDateTime) {
    let start = local_naive_to_utc(hour, Shanghai);
    let end = local_naive_to_utc(
        hour.checked_add_signed(ChronoDuration::hours(1))
            .expect("same-bucket summary timeseries end"),
        Shanghai,
    );
    let Json(response) = fetch_timeseries_from_hourly_rollups(
        state,
        TimeseriesQuery {
            range: "ignored".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        },
        Shanghai,
        InvocationSourceScope::ProxyOnly,
        RangeWindow {
            start,
            end,
            display_end: end,
            duration: end - start,
        },
        TimeseriesBucketSelection {
            bucket_seconds: 3_600,
            effective_bucket: "1h".to_string(),
            available_buckets: vec!["1h".to_string()],
            bucket_limited_to_daily: false,
        },
    )
    .await
    .expect("fetch same-bucket summary timeseries");
    let point = response
        .points
        .iter()
        .find(|point| point.bucket_start == format_utc_iso(start))
        .expect("same-bucket summary timeseries bucket should exist");
    assert_eq!(point.total_count, 2);
    assert_eq!(point.success_count, 2);
    assert_eq!(point.failure_count, 0);
    assert_eq!(point.total_tokens, 30);
    assert_f64_close(point.total_cost, 0.30);
}

#[tokio::test]
pub(crate) async fn all_time_summary_skips_double_count_for_readable_materialized_archive_when_same_bucket_sibling_is_unreadable()
 {
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.invocation_max_days = 7;
    let state = test_state_from_config(config, true).await;

    let archived_hour_local = (Utc::now().with_timezone(&Shanghai).date_naive()
        - ChronoDuration::days(10))
    .and_hms_opt(7, 0, 0)
    .expect("valid same-bucket summary hour");
    let archived_readable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(5))
            .expect("same-bucket readable summary archived time"),
    );
    let archived_unreadable_at = format_naive(
        archived_hour_local
            .checked_add_signed(ChronoDuration::minutes(25))
            .expect("same-bucket unreadable summary archived time"),
    );

    let readable_archive_path = seed_materialized_summary_archive(
        &state,
        "summary-same-bucket-unreadable-sibling-materialized",
        &archived_readable_at,
        10,
        0.10,
        100.0,
    )
    .await;
    let unreadable_archive_path = seed_materialized_summary_archive(
        &state,
        "summary-same-bucket-unreadable-sibling-broken",
        &archived_unreadable_at,
        20,
        0.20,
        120.0,
    )
    .await;

    seed_same_bucket_summary_rollup(
        &state,
        &archived_readable_at,
        [&readable_archive_path, &unreadable_archive_path],
    )
    .await;
    fs::write(&unreadable_archive_path, b"not-a-gzip-archive")
        .expect("corrupt unreadable same-bucket summary archive");

    let summary = fetch_test_summary(state.clone(), "all").await;
    assert_two_success_summary(&summary);
    assert_same_bucket_summary_timeseries(state, archived_hour_local).await;
}

use super::*;
