
#[tokio::test]
async fn exact_record_limit_test_override_is_task_scoped_and_defaults_to_production_limit() {
    assert_eq!(
        summary_projection_exact_record_limit(),
        SUMMARY_PROJECTION_MAX_EXACT_RECORDS
    );

    let (first, second) = tokio::join!(
        with_summary_projection_test_exact_record_limit(8, async {
            tokio::task::yield_now().await;
            summary_projection_exact_record_limit()
        }),
        with_summary_projection_test_exact_record_limit(13, async {
            tokio::task::yield_now().await;
            summary_projection_exact_record_limit()
        }),
    );

    assert_eq!(first, 8);
    assert_eq!(second, 13);
    assert_eq!(
        summary_projection_exact_record_limit(),
        SUMMARY_PROJECTION_MAX_EXACT_RECORDS
    );
}

#[tokio::test]
async fn summary_all_time_rejects_conflicting_completed_manifest_identity_for_one_path() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    for (id, month_key, sha256) in [
        (1_i64, "2026-08", "summary-conflicting-sha-a"),
        (2_i64, "2026-09", "summary-conflicting-sha-b"),
    ] {
        sqlx::query(
                "INSERT INTO archive_batches
                 (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
                 VALUES (?1, 'codex_invocations', ?2, '/archive/shared.sqlite.gz', ?3, 1, 'completed', 'unknown')",
            )
            .bind(id)
            .bind(month_key)
            .bind(sha256)
            .execute(&pool)
            .await
            .expect("insert conflicting completed manifest");
    }

    let error = match load_summary_projection_all_time_archive_scan_paths(&pool).await {
        Ok(_) => panic!("conflicting archive identities must fail closed"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("archive identity is ambiguous"));
}

#[tokio::test]
async fn summary_all_time_rejects_duplicate_completed_manifest_identity_for_one_path() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");
    for (id, month_key) in [(1_i64, "2026-08"), (2_i64, "2026-09")] {
        sqlx::query(
            "INSERT INTO archive_batches
                 (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
                 VALUES (?1, 'codex_invocations', ?2, '/archive/duplicate.sqlite.gz',
                         'summary-duplicate-sha', 1, 'completed', 'unknown')",
        )
        .bind(id)
        .bind(month_key)
        .execute(&pool)
        .await
        .expect("insert duplicate completed manifest");
    }

    let error = match load_summary_projection_all_time_archive_scan_paths(&pool).await {
        Ok(_) => panic!("duplicate archive identities must fail closed"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("archive identity is duplicated"));
}

#[tokio::test]
async fn summary_recent_proof_identity_requires_bounded_manifest_coverage() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("current schema");

    sqlx::query(
        "INSERT INTO archive_batches
             (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind)
             VALUES (1, 'codex_invocations', '2026-09', '/archive/unknown.sqlite.gz',
                     'summary-v2-unknown-range', 1, 'completed', 'unknown')",
    )
    .execute(&pool)
    .await
    .expect("seed unknown-range archive manifest");
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_v2_proof
             (archive_batch_id, manifest_sha256, page_count, row_count, semantic_sha256)
             VALUES (1, 'summary-v2-unknown-range', 1, 1, 'semantic-proof')",
    )
    .execute(&pool)
    .await
    .expect("seed unknown-range V2 proof");

    let now = Utc::now();
    let ids = load_summary_v2_archive_proof_identities_in_range(
        &pool,
        now - ChronoDuration::days(30),
        now,
    )
    .await
    .expect("load recent proof identities");
    assert!(
        ids.is_empty(),
        "an unbounded manifest must not repeatedly trigger recent overlay publication"
    );

    let coverage_start = crate::stats::db_occurred_at_lower_bound(now - ChronoDuration::hours(1));
    let coverage_end = crate::db_occurred_at_upper_bound(now);
    sqlx::query(
        "INSERT INTO archive_batches
             (id, dataset, month_key, file_path, sha256, row_count, status, summary_source_kind,
              coverage_start_at, coverage_end_at)
             VALUES (2, 'codex_invocations', '2026-09', '/archive/bounded.sqlite.gz',
                     'summary-v2-bounded-range', 1, 'completed', 'unknown', ?1, ?2)",
    )
    .bind(coverage_start)
    .bind(coverage_end)
    .execute(&pool)
    .await
    .expect("seed bounded archive manifest");
    sqlx::query(
        "INSERT INTO summary_archive_snapshot_v2_proof
             (archive_batch_id, manifest_sha256, page_count, row_count, semantic_sha256)
             VALUES (2, 'summary-v2-bounded-range', 1, 1, 'semantic-proof')",
    )
    .execute(&pool)
    .await
    .expect("seed bounded V2 proof");

    let ids = load_summary_v2_archive_proof_identities_in_range(
        &pool,
        now - ChronoDuration::days(30),
        now,
    )
    .await
    .expect("reload recent proof identities");
    assert!(ids.contains(&(2, "summary-v2-bounded-range".to_string())));
    pool.close().await;
}

#[tokio::test]
async fn summary_projection_paged_manifests_include_minimum_legacy_id() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    sqlx::query(
        "CREATE TABLE archive_batches ( \
                 id INTEGER PRIMARY KEY, \
                 dataset TEXT NOT NULL, \
                 status TEXT NOT NULL, \
                 file_path TEXT NOT NULL, \
                 coverage_start_at TEXT, \
                 coverage_end_at TEXT, \
                 historical_rollups_materialized_at TEXT, \
                 upstream_activity_manifest_refreshed_at TEXT, \
                 summary_source_kind TEXT NOT NULL DEFAULT 'unknown' \
             )",
    )
    .execute(&pool)
    .await
    .expect("create archive manifest table");
    for (id, path) in [(i64::MIN, "minimum-id.sqlite.gz"), (0, "zero-id.sqlite.gz")] {
        sqlx::query(
            "INSERT INTO archive_batches \
                 (id, dataset, status, file_path) \
                 VALUES (?1, 'codex_invocations', 'completed', ?2)",
        )
        .bind(id)
        .bind(path)
        .execute(&pool)
        .await
        .expect("insert legacy manifest id");
    }
    let page = load_summary_projection_boundary_manifest_page(&pool, None, None, 0)
        .await
        .expect("load first keyset page without a sentinel cursor");
    let paths = page
        .archives
        .iter()
        .map(|archive| archive.file_path())
        .collect::<HashSet<_>>();
    assert!(paths.contains("minimum-id.sqlite.gz"));
    assert!(paths.contains("zero-id.sqlite.gz"));
}

async fn summary_projection_replay_coverage_fixture() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    sqlx::query(
        "CREATE TABLE archive_batches ( \
                 id INTEGER PRIMARY KEY, \
                 dataset TEXT NOT NULL, \
                 status TEXT NOT NULL, \
                 file_path TEXT NOT NULL, \
                 sha256 TEXT, \
                 historical_rollups_materialized_at TEXT, \
                 summary_source_kind TEXT NOT NULL DEFAULT 'unknown' \
             )",
    )
    .execute(&pool)
    .await
    .expect("create archive manifest table");
    sqlx::query(
        "CREATE TABLE hourly_rollup_archive_replay ( \
                 target TEXT NOT NULL, \
                 dataset TEXT NOT NULL, \
                 file_path TEXT NOT NULL, \
                 archive_sha256 TEXT \
             )",
    )
    .execute(&pool)
    .await
    .expect("create archive replay table");
    sqlx::query(
        "INSERT INTO archive_batches \
             (id, dataset, status, file_path, sha256) \
             VALUES (1, 'codex_invocations', 'completed', 'replacement.sqlite.gz', 'new-sha')",
    )
    .execute(&pool)
    .await
    .expect("insert replacement archive manifest");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay \
                 (target, dataset, file_path, archive_sha256) \
                 VALUES (?1, 'codex_invocations', 'replacement.sqlite.gz', 'old-sha')",
        )
        .bind(target)
        .execute(&pool)
        .await
        .expect("insert stale replay marker");
    }
    pool
}

#[tokio::test]
async fn summary_projection_replay_coverage_requires_a_nonempty_matching_manifest_sha() {
    let pool = summary_projection_replay_coverage_fixture().await;

    let stale_coverage = load_summary_projection_archive_replay_coverage(
        &pool,
        &["replacement.sqlite.gz".to_string()],
    )
    .await
    .expect("read stale replay coverage");
    assert!(
        !stale_coverage
            .get("replacement.sqlite.gz")
            .copied()
            .unwrap_or_default()
            .supports_unavailable_archive(),
        "a replacement manifest must not inherit old replay proof"
    );

    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = 'new-sha' \
             WHERE dataset = 'codex_invocations' AND file_path = 'replacement.sqlite.gz'",
    )
    .execute(&pool)
    .await
    .expect("refresh replay markers for replacement manifest");
    let current_coverage = load_summary_projection_archive_replay_coverage(
        &pool,
        &["replacement.sqlite.gz".to_string()],
    )
    .await
    .expect("read current replay coverage");
    assert!(
        current_coverage
            .get("replacement.sqlite.gz")
            .copied()
            .unwrap_or_default()
            .supports_unavailable_archive(),
        "current replay proof must cover every compact response dimension"
    );

    sqlx::query(
        "UPDATE archive_batches SET historical_rollups_materialized_at = datetime('now') \
             WHERE file_path = 'replacement.sqlite.gz'",
    )
    .execute(&pool)
    .await
    .expect("mark archive manifest materialized");
    sqlx::query(
        "UPDATE hourly_rollup_archive_replay SET archive_sha256 = NULL \
             WHERE dataset = 'codex_invocations' AND file_path = 'replacement.sqlite.gz'",
    )
    .execute(&pool)
    .await
    .expect("restore legacy replay markers without a manifest hash");
    let identityless_coverage = load_summary_projection_archive_replay_coverage(
        &pool,
        &["replacement.sqlite.gz".to_string()],
    )
    .await
    .expect("read identityless replay coverage");
    assert!(
        !identityless_coverage
            .get("replacement.sqlite.gz")
            .copied()
            .unwrap_or_default()
            .supports_unavailable_archive(),
        "materialization must not make an identityless replay marker exact"
    );

    for manifest_sha256 in [None, Some("")] {
        sqlx::query(
            "UPDATE archive_batches SET sha256 = ?1 \
                 WHERE file_path = 'replacement.sqlite.gz'",
        )
        .bind(manifest_sha256)
        .execute(&pool)
        .await
        .expect("remove immutable manifest identity");
        let manifestless_coverage = load_summary_projection_archive_replay_coverage(
            &pool,
            &["replacement.sqlite.gz".to_string()],
        )
        .await
        .expect("read replay coverage without a manifest identity");
        assert!(
            !manifestless_coverage
                .get("replacement.sqlite.gz")
                .copied()
                .unwrap_or_default()
                .supports_unavailable_archive(),
            "a null or blank manifest identity must not make a materialized archive exact"
        );
    }
}

#[test]
fn summary_projection_terminal_coverage_includes_occurred_at() {
    let mut projection = SummaryProjection::default();
    projection
        .persisted_live_terminal_invoke_ids
        .insert("replayed-terminal\u{0}2026-08-23 10:00:00".to_string());

    assert!(
        projection.contains_persisted_live_terminal("replayed-terminal", "2026-08-23 10:00:00")
    );
    assert!(
        !projection.contains_persisted_live_terminal("replayed-terminal", "2026-08-23 10:01:00")
    );
}

struct SummaryArchiveRangesFixture {
    end: DateTime<Utc>,
    exact_range: ExactUtcRange,
    protected: HashSet<i64>,
    known_accounts: HashSet<i64>,
    usage: UsageBreakdownResponse,
    totals: HashMap<(i64, Option<i64>), StatsTotals>,
    usage_totals: HashMap<(i64, Option<i64>), UsageBreakdownResponse>,
    missing_account_bucket: i64,
}

fn summary_projection_archive_ranges_fixture() -> SummaryArchiveRangesFixture {
    let end = Utc
        .timestamp_opt(1_800_000_000, 0)
        .single()
        .expect("valid range end");
    let exact_range = ExactUtcRange {
        start: end - ChronoDuration::hours(3),
        end,
    };
    let protected = HashSet::from([align_bucket_epoch(end.timestamp(), 3_600, 0)]);
    let known_accounts = HashSet::from([42_i64]);
    let usage = UsageBreakdownResponse {
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        output_tokens: 0,
        costs: None,
        models: Vec::new(),
    };
    let mut totals = HashMap::new();
    let mut usage_totals = HashMap::new();
    for bucket in [
        align_bucket_epoch((end - ChronoDuration::hours(3)).timestamp(), 3_600, 0),
        align_bucket_epoch((end - ChronoDuration::hours(2)).timestamp(), 3_600, 0),
    ] {
        totals.insert((bucket, None), StatsTotals::default());
        usage_totals.insert((bucket, None), usage.clone());
        totals.insert((bucket, Some(42)), StatsTotals::default());
        usage_totals.insert((bucket, Some(42)), usage.clone());
    }
    let missing_account_bucket =
        align_bucket_epoch((end - ChronoDuration::hours(1)).timestamp(), 3_600, 0);
    totals.insert((missing_account_bucket, None), StatsTotals::default());
    usage_totals.insert((missing_account_bucket, None), usage.clone());
    SummaryArchiveRangesFixture {
        end,
        exact_range,
        protected,
        known_accounts,
        usage,
        totals,
        usage_totals,
        missing_account_bucket,
    }
}

fn assert_summary_projection_archive_range_boundaries(end: DateTime<Utc>) {
    assert!(
        summary_projection_boundary_buckets(end).contains(&align_bucket_epoch(
            (end - ChronoDuration::days(14)).timestamp(),
            3_600,
            0,
        ))
    );
    for hours in 1..=SUMMARY_PROJECTION_MIN_EXACT_HORIZON.num_hours() {
        assert!(
            summary_projection_boundary_buckets(end).contains(&align_bucket_epoch(
                (end - ChronoDuration::hours(hours)).timestamp(),
                3_600,
                0,
            )),
            "every legal <=48h duration start hour must retain exact archive detail"
        );
    }
}

fn assert_summary_projection_archive_range_missing_scope(
    fixture: &mut SummaryArchiveRangesFixture,
) {
    let ranges = summary_projection_archive_exact_ranges(
        true,
        fixture.exact_range,
        &fixture.protected,
        &fixture.totals,
        &fixture.usage_totals,
        &fixture.known_accounts,
    );
    assert!(ranges.iter().any(|range| {
        range.start
            <= Utc
                .timestamp_opt(fixture.missing_account_bucket, 0)
                .unwrap()
            && range.end
                >= Utc
                    .timestamp_opt(fixture.missing_account_bucket + 3_600, 0)
                    .unwrap()
    }));
    let mut live_replacement_buckets = HashSet::new();
    summary_projection_extend_exact_buckets_for_ranges(&mut live_replacement_buckets, ranges)
        .expect("missing scope fallback remains within the exact bucket budget");
    assert!(
        live_replacement_buckets.contains(&fixture.missing_account_bucket),
        "a raw archive fallback bucket must also hydrate its persisted/live replacement"
    );
    fixture.totals.insert(
        (fixture.missing_account_bucket, Some(42)),
        StatsTotals::default(),
    );
    fixture.usage_totals.insert(
        (fixture.missing_account_bucket, Some(42)),
        fixture.usage.clone(),
    );
    let covered_ranges = summary_projection_archive_exact_ranges(
        true,
        fixture.exact_range,
        &fixture.protected,
        &fixture.totals,
        &fixture.usage_totals,
        &fixture.known_accounts,
    );
    assert!(!covered_ranges.iter().any(|range| {
        range.start
            <= Utc
                .timestamp_opt(fixture.missing_account_bucket, 0)
                .unwrap()
            && range.end
                >= Utc
                    .timestamp_opt(fixture.missing_account_bucket + 3_600, 0)
                    .unwrap()
    }));
    let replayed_ranges = summary_projection_archive_exact_ranges_with_coverage(
        true,
        SummaryProjectionArchiveReplayFlags {
            overall: Some(true),
            account: Some(true),
            usage: Some(true),
        },
        fixture.exact_range,
        &HashSet::new(),
        &fixture.totals,
        &fixture.usage_totals,
        &fixture.known_accounts,
    );
    assert!(replayed_ranges.is_empty());
}

fn assert_summary_projection_archive_range_replay_guards(
    fixture: &mut SummaryArchiveRangesFixture,
) {
    fixture
        .totals
        .remove(&(fixture.missing_account_bucket, Some(42)));
    fixture
        .usage_totals
        .remove(&(fixture.missing_account_bucket, Some(42)));
    let paged_missing_account_ranges = summary_projection_archive_exact_ranges_with_coverage(
        true,
        SummaryProjectionArchiveReplayFlags {
            overall: Some(true),
            account: None,
            usage: None,
        },
        fixture.exact_range,
        &HashSet::new(),
        &fixture.totals,
        &fixture.usage_totals,
        &fixture.known_accounts,
    );
    assert!(paged_missing_account_ranges.iter().any(|range| {
        range.start
            <= Utc
                .timestamp_opt(fixture.missing_account_bucket, 0)
                .unwrap()
            && range.end
                >= Utc
                    .timestamp_opt(fixture.missing_account_bucket + 3_600, 0)
                    .unwrap()
    }));
    fixture.totals.insert(
        (fixture.missing_account_bucket, Some(42)),
        StatsTotals::default(),
    );
    fixture.usage_totals.insert(
        (fixture.missing_account_bucket, Some(42)),
        fixture.usage.clone(),
    );
    let stale_manifest_ranges = summary_projection_archive_exact_ranges_with_coverage(
        true,
        SummaryProjectionArchiveReplayFlags {
            overall: Some(true),
            account: Some(false),
            usage: Some(false),
        },
        fixture.exact_range,
        &HashSet::new(),
        &fixture.totals,
        &fixture.usage_totals,
        &fixture.known_accounts,
    );
    assert!(!stale_manifest_ranges.is_empty());
    let missing_global_replay_ranges = summary_projection_archive_exact_ranges_with_coverage(
        true,
        SummaryProjectionArchiveReplayFlags {
            overall: Some(false),
            account: Some(true),
            usage: Some(true),
        },
        fixture.exact_range,
        &HashSet::new(),
        &fixture.totals,
        &fixture.usage_totals,
        &fixture.known_accounts,
    );
    assert!(
        missing_global_replay_ranges.iter().any(|range| {
            range.start
                <= Utc
                    .timestamp_opt(fixture.missing_account_bucket, 0)
                    .unwrap()
                && range.end
                    >= Utc
                        .timestamp_opt(fixture.missing_account_bucket + 3_600, 0)
                        .unwrap()
        }),
        "a partial global rollup cannot suppress archive exact rows without overall replay"
    );
    let archive_only_ranges = summary_projection_archive_exact_ranges(
        true,
        fixture.exact_range,
        &HashSet::new(),
        &fixture.totals,
        &fixture.usage_totals,
        &HashSet::new(),
    );
    assert!(!archive_only_ranges.is_empty());
}

#[test]
fn summary_projection_archive_ranges_include_missing_scope_coverage() {
    let mut fixture = summary_projection_archive_ranges_fixture();
    assert_summary_projection_archive_range_boundaries(fixture.end);
    assert_summary_projection_archive_range_missing_scope(&mut fixture);
    assert_summary_projection_archive_range_replay_guards(&mut fixture);
}

#[test]
fn summary_projection_marks_partial_replay_as_scope_exact_replacement() {
    let start = Utc
        .timestamp_opt(1_800_000_000, 0)
        .single()
        .expect("valid range start");
    let exact_range = ExactUtcRange {
        start: start + ChronoDuration::minutes(15),
        end: start + ChronoDuration::hours(1) + ChronoDuration::minutes(45),
    };
    let first_bucket = align_bucket_epoch(start.timestamp(), 3_600, 0);
    let last_bucket = first_bucket + 3_600;
    let mut global_totals = HashSet::new();
    let mut account_totals = HashSet::new();
    let mut global_usage = HashSet::new();
    let mut account_usage = HashSet::new();
    summary_projection_mark_exact_replacement_buckets(SummaryProjectionExactReplacementInput {
        archive_has_materialized_rollups: true,
        replay_coverage: SummaryProjectionArchiveReplayCoverage {
            overall: false,
            account_stats: true,
            usage_breakdown: false,
        },
        exact_range,
        _protected_boundary_buckets: &HashSet::new(),
        exact_global_total_rollup_buckets: &mut global_totals,
        exact_account_total_rollup_buckets: &mut account_totals,
        exact_global_usage_rollup_buckets: &mut global_usage,
        exact_account_usage_rollup_buckets: &mut account_usage,
    })
    .expect("partial replay range remains within the exact bucket budget");

    assert!(global_totals.contains(&first_bucket));
    assert!(global_totals.contains(&last_bucket));
    assert!(!account_totals.contains(&first_bucket));
    assert!(global_usage.contains(&first_bucket));
    assert!(global_usage.contains(&last_bucket));
    assert!(account_usage.contains(&first_bucket));
    assert!(account_usage.contains(&last_bucket));
}

#[test]
fn summary_projection_rejects_wide_exact_fallback_before_bucket_expansion() {
    let start = Utc
        .timestamp_opt(1_800_000_000, 0)
        .single()
        .expect("valid exact range start");
    let range = ExactUtcRange {
        start,
        end: start + ChronoDuration::hours(SUMMARY_PROJECTION_MAX_EXACT_BUCKETS as i64 + 1),
    };
    let error = summary_projection_exact_range_fits_bucket_budget(range)
        .expect_err("one bucket beyond the budget must fail before expansion");
    assert!(error.to_string().contains("exact range bucket budget"));
}

#[test]
fn summary_projection_all_time_requires_exact_horizon_coverage_for_partial_archive() {
    let start = Utc
        .timestamp_opt(1_800_000_000, 0)
        .single()
        .expect("valid horizon start");
    let horizon = ExactUtcRange {
        start,
        end: start + ChronoDuration::days(30),
    };
    let older_archive = crate::stats::ArchiveBatchPathRow::with_coverage(
        "archive.sqlite.gz".to_string(),
        Some((start - ChronoDuration::hours(1)).to_rfc3339()),
        Some((start + ChronoDuration::hours(1)).to_rfc3339()),
    );
    assert!(
        !summary_projection_archive_is_fully_within_exact_horizon(&older_archive, horizon),
        "all-time cannot use a bounded replacement for an archive extending before it"
    );
}

#[test]
fn summary_projection_archive_overlap_avoids_unrelated_old_batch_io() {
    let requested_range = ExactUtcRange {
        start: Utc
            .with_ymd_and_hms(2026, 8, 20, 0, 0, 0)
            .single()
            .expect("valid request start"),
        end: Utc
            .with_ymd_and_hms(2026, 8, 20, 2, 0, 0)
            .single()
            .expect("valid request end"),
    };
    let old_archive = crate::stats::ArchiveBatchPathRow::with_coverage(
        "old",
        Some("2026-07-01T00:00:00Z".to_string()),
        Some("2026-07-02T00:00:00Z".to_string()),
    );
    assert!(summary_projection_archive_overlap_range(&old_archive, requested_range).is_none());

    let overlapping_archive = crate::stats::ArchiveBatchPathRow::with_coverage(
        "overlap",
        Some("2026-08-19T23:00:00Z".to_string()),
        Some("2026-08-20T01:00:00Z".to_string()),
    );
    let overlap = summary_projection_archive_overlap_range(&overlapping_archive, requested_range)
        .expect("archive overlaps request");
    assert_eq!(overlap.start, requested_range.start);
    assert_eq!(
        overlap.end,
        Utc.with_ymd_and_hms(2026, 8, 20, 1, 0, 1)
            .single()
            .expect("valid overlap end")
    );
}

#[tokio::test]
async fn summary_projection_discovers_archive_only_account_before_coverage_planning() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    sqlx::query("CREATE TABLE codex_invocations (occurred_at TEXT NOT NULL, payload TEXT)")
        .execute(&pool)
        .await
        .expect("create archive preview table");
    sqlx::query(
        "INSERT INTO codex_invocations (occurred_at, payload) VALUES (?1, ?2), (?1, ?3), (?1, ?4)",
    )
    .bind("2026-07-28 00:26:54")
    .bind(r#"{"upstreamAccountId":77}"#)
    .bind(r#"{"upstreamAccountId":0}"#)
    .bind("not-json")
    .execute(&pool)
    .await
    .expect("insert archive-only account rows");

    let accounts = load_summary_projection_archive_account_ids(
        &pool,
        ExactUtcRange {
            start: Utc
                .with_ymd_and_hms(2026, 7, 27, 16, 0, 0)
                .single()
                .expect("valid range start"),
            end: Utc
                .with_ymd_and_hms(2026, 7, 27, 17, 0, 0)
                .single()
                .expect("valid range end"),
        },
    )
    .await
    .expect("discover archive-only account");
    assert_eq!(accounts, HashSet::from([77_i64]));
}

#[tokio::test]
async fn request_compression_prefers_the_final_pool_attempt_without_using_earlier_attempts() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite pool");
    sqlx::query(
            "CREATE TABLE codex_invocations (invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, payload TEXT)",
        )
            .execute(&pool)
            .await
            .expect("create invocations table");
    sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, status TEXT, upstream_request_compression_algorithm TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create upstream attempts table");

    sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, payload) VALUES ('pool-retry', '2026-07-30 10:00:00', '{\"requestCompressionAlgorithm\":\"gzip\"}'), ('pool-retry', '2026-07-30 10:01:00', '{\"requestCompressionAlgorithm\":\"gzip\"}'), ('direct', '2026-07-30 10:00:00', '{\"requestCompressionAlgorithm\":\"gzip\"}'), ('final-unknown', '2026-07-30 10:00:00', '{\"requestCompressionAlgorithm\":\"gzip\"}')",
        )
        .execute(&pool)
        .await
        .expect("insert invocations");
    sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, attempt_index, status, upstream_request_compression_algorithm) VALUES (1, 'pool-retry', '2026-07-30 10:00:00', 1, 'success', 'br'), (2, 'pool-retry', '2026-07-30 10:00:00', 2, 'success', 'zstd'), (3, 'pool-retry', '2026-07-30 10:01:00', 1, 'success', 'identity'), (4, 'final-unknown', '2026-07-30 10:00:00', 1, 'success', 'deflate'), (5, 'final-unknown', '2026-07-30 10:00:00', 2, 'http_failure', NULL), (6, 'pool-retry', '2026-07-30 10:00:00', 3, 'budget_exhausted_final', NULL)",
        )
        .execute(&pool)
        .await
        .expect("insert upstream attempts");

    let compression_sql =
        invocation_request_compression_algorithm_with_attempt_fallback_sql("codex_invocations");
    let query = format!(
        "SELECT invoke_id, occurred_at, {compression_sql} AS request_compression_algorithm FROM codex_invocations ORDER BY invoke_id, occurred_at"
    );
    let rows = sqlx::query_as::<_, (String, String, Option<String>)>(&query)
        .fetch_all(&pool)
        .await
        .expect("query request compression");

    assert_eq!(
        rows,
        vec![
            (
                "direct".to_string(),
                "2026-07-30 10:00:00".to_string(),
                Some("gzip".to_string())
            ),
            (
                "final-unknown".to_string(),
                "2026-07-30 10:00:00".to_string(),
                None
            ),
            (
                "pool-retry".to_string(),
                "2026-07-30 10:00:00".to_string(),
                Some("zstd".to_string())
            ),
            (
                "pool-retry".to_string(),
                "2026-07-30 10:01:00".to_string(),
                Some("identity".to_string())
            ),
        ]
    );
}

async fn hydrate_summary_projection_fixture(state: &Arc<AppState>) {
    sqlx::query(
            "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-projection-fixture', datetime('now'), 'proxy', 'success', 17, 1.25, '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("insert summary projection fixture");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("hydrate summary projection fixture");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::AllTime)
        .await
        .expect("hydrate summary projection fixture all-time coverage");
}

#[tokio::test]
async fn summary_projection_bootstrap_publishes_exact_rolling_before_all_time_reconciliation() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = db_occurred_at_lower_bound(Utc::now());
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-bootstrap-rolling', ?1, 'proxy', 'success', 17, 1.25, \
                     '{\"upstreamAccountId\":42}', '', 'full')",
        )
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("insert bootstrap summary fixture");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create summary projection interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::AfterAllTimeArchiveDiscovery,
    );
    let state_for_hydration = state.clone();
    let hydration = tokio::spawn(async move {
        hydrate_summary_snapshots_with_deadline(
            state_for_hydration.as_ref(),
            SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        )
        .await
    });
    tokio::pin!(hydration);

    tokio::select! {
        result = &mut hydration => {
            clear_summary_projection_test_interleave();
            result
                .expect("run bootstrap hydration")
                .expect("bootstrap hydration must publish the rolling projection");
        }
        _ = interleave.wait_for_writer() => {
            interleave.resume_build();
            let _ = hydration.await;
            clear_summary_projection_test_interleave();
            panic!("startup hydration must publish rolling coverage without waiting for all-time reconciliation");
        }
    }

    state.pool.close().await;
    for window in ["current", "1d", "7d", "30d", "today"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("bootstrap projection must serve rolling Summary from memory");
        assert_eq!(response.total_count, 1, "{window} response");
        assert_eq!(response.total_tokens, 17, "{window} response");
    }
    let all_time = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(all_time, Err(ApiError::Unavailable(_))));
}

#[tokio::test]
async fn summary_projection_live_tail_checkpoint_round_trips_target_and_cursor() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let checkpoint = SummaryLiveTailReconciliationCheckpoint {
        format_version: 1,
        recovery_epoch: 7,
        base_projection_revision: 11,
        target_terminal_watermark: 23,
        target_source_cursor: 31,
        next_source_cursor: 29,
        state: "deferred".to_string(),
    };
    store_summary_live_tail_reconciliation_checkpoint(&state.pool, checkpoint.clone())
        .await
        .expect("store live-tail checkpoint");
    assert_eq!(
        load_summary_live_tail_reconciliation_checkpoint(&state.pool)
            .await
            .expect("load live-tail checkpoint"),
        Some(checkpoint)
    );
}

async fn assert_summary_projection_windows_after_memory_refresh(state: Arc<AppState>) {
    for window in ["current", "1d", "7d", "30d", "today"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("RollingDelta must serve the exact memory projection");
        assert_eq!(response.total_count, 2, "{window} exact count");
        assert_eq!(response.total_tokens, 40, "{window} exact tokens");
    }
    let Json(all_time) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("completed all-time proof must serve the exact projection");
    assert_eq!(all_time.total_count, 2);
    assert_eq!(all_time.total_tokens, 40);
}

#[tokio::test]
async fn summary_projection_rolling_delta_publishes_without_full_live_admission() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(2));
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-delta-base', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("seed immutable Summary projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");

    let mut terminal = summary_projection_test_invocation();
    terminal.id = 913_001;
    terminal.invoke_id = "summary-delta-committed-terminal".to_string();
    terminal.occurred_at = db_occurred_at_lower_bound(Utc::now());
    terminal.source = SOURCE_PROXY.to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(23);
    terminal.output_tokens = Some(11);
    terminal.cost = Some(2.5);
    terminal.upstream_account_id = Some(42);
    sqlx::query(
            "INSERT INTO codex_invocations \
             (id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost, payload, raw_response, detail_level) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', 'full')",
        )
        .bind(terminal.id)
        .bind(&terminal.invoke_id)
        .bind(&terminal.occurred_at)
        .bind(&terminal.source)
        .bind(terminal.status.as_deref())
        .bind(terminal.total_tokens)
        .bind(terminal.output_tokens)
        .bind(terminal.cost)
        .bind(json!({ "upstreamAccountId": 42 }).to_string())
        .execute(&state.pool)
        .await
        .expect("persist terminal before its journal ACK");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("materialize committed terminal delta");
    state
        .subscription_hub
        .acknowledge_summary_delta(delta)
        .await;

    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("enable full-build probe");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::AfterRollupLoad,
    );
    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("committed journal must renew rolling projection");
    assert!(
        interleave.build_modes().is_empty(),
        "RollingDelta must not enter full live admission"
    );
    clear_summary_projection_test_interleave();

    state.pool.close().await;
    assert_summary_projection_windows_after_memory_refresh(state).await;
}

#[tokio::test]
async fn summary_projection_replayed_delta_publishes_without_full_live_admission() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(2));
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-replay-base', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("seed immutable Summary projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");

    let mut terminal = summary_projection_test_invocation();
    terminal.id = 913_002;
    terminal.invoke_id = "summary-replayed-terminal".to_string();
    terminal.occurred_at = db_occurred_at_lower_bound(Utc::now());
    terminal.source = SOURCE_PROXY.to_string();
    terminal.status = Some("success".to_string());
    terminal.live_phase = None;
    terminal.total_tokens = Some(23);
    terminal.output_tokens = Some(11);
    terminal.cost = Some(2.5);
    terminal.upstream_account_id = Some(42);
    sqlx::query(
            "INSERT INTO codex_invocations \
             (id, invoke_id, occurred_at, source, status, total_tokens, output_tokens, cost, payload, raw_response, detail_level) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', 'full')",
        )
        .bind(terminal.id)
        .bind(&terminal.invoke_id)
        .bind(&terminal.occurred_at)
        .bind(&terminal.source)
        .bind(terminal.status.as_deref())
        .bind(terminal.total_tokens)
        .bind(terminal.output_tokens)
        .bind(terminal.cost)
        .bind(json!({ "upstreamAccountId": 42 }).to_string())
        .execute(&state.pool)
        .await
        .expect("persist replayed terminal before its journal ACK");
    let delta = apply_dashboard_activity_terminal_record(state.as_ref(), &terminal)
        .await
        .terminal_delta
        .expect("materialize committed replay delta");
    state
        .subscription_hub
        .acknowledge_replayed_summary_delta(delta)
        .await;

    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("enable full-build probe");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::AfterRollupLoad,
    );
    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("replayed journal delta must renew rolling projection");
    assert!(
        interleave.build_modes().is_empty(),
        "replayed delta must not enter full live admission"
    );
    clear_summary_projection_test_interleave();

    state.pool.close().await;
    for window in ["current", "1d", "7d", "30d", "today"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("replayed delta must serve the exact memory projection");
        assert_eq!(response.total_count, 2, "{window} exact count");
        assert_eq!(response.total_tokens, 40, "{window} exact tokens");
    }
}

struct SummaryProjectionDeltaGapFixture {
    projection: Arc<SummaryProjection>,
    gap: DeltaGapProof,
    newer_delta: DashboardActivityTerminalDelta,
}

async fn summary_projection_delta_gap_fixture() -> SummaryProjectionDeltaGapFixture {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let newest = db_occurred_at_lower_bound(Utc::now());
    for (index, occurred_at) in [
        newest,
        db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(1)),
        db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(2)),
    ]
    .into_iter()
    .enumerate()
    {
        sqlx::query(
                "INSERT INTO codex_invocations \
                 (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
                 VALUES (?1, ?2, 'proxy', 'success', 1, 0.01, '{\"upstreamAccountId\":42}', '', 'full')",
            )
            .bind(format!("summary-delta-gap-rank-{index}"))
            .bind(occurred_at)
            .execute(&state.pool)
            .await
            .expect("seed ordered Summary current record");
    }
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish Summary projection");
    let projection = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("published projection");
    let gap = DeltaGapProof {
        cursor: SummaryDeltaCursor(4),
        terminal_sequence: Some(4),
        upstream_account_id: Some(42),
        occurred_at: db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(3)),
        row_id: Some(i64::MAX),
        invoke_id: Some("summary-delta-gap-tail".to_string()),
    };
    let mut delta_record = summary_projection_test_invocation();
    delta_record.id = 0;
    delta_record.invoke_id = "summary-delta-gap-tail".to_string();
    delta_record.occurred_at = db_occurred_at_lower_bound(Utc::now() - ChronoDuration::minutes(2));
    delta_record.status = Some("success".to_string());
    delta_record.live_phase = None;
    delta_record.upstream_account_id = Some(42);
    let newer_delta = apply_dashboard_activity_terminal_record(state.as_ref(), &delta_record)
        .await
        .terminal_delta
        .expect("materialize committed delta ordering fixture");
    SummaryProjectionDeltaGapFixture {
        projection,
        gap,
        newer_delta,
    }
}

#[tokio::test]
async fn summary_projection_delta_gap_respects_current_rank_and_account_scope() {
    let SummaryProjectionDeltaGapFixture {
        projection,
        gap,
        newer_delta,
    } = summary_projection_delta_gap_fixture().await;

    assert!(
        !summary_delta_gap_affects_selection(
            projection.as_ref(),
            std::slice::from_ref(&gap),
            std::slice::from_ref(&newer_delta),
            &SummaryWindow::Current(2),
            Shanghai,
            None,
        ),
        "a gap beyond the requested current prefix must remain selection-local"
    );
    assert!(
        !summary_delta_gap_affects_selection(
            projection.as_ref(),
            std::slice::from_ref(&gap),
            std::slice::from_ref(&newer_delta),
            &SummaryWindow::Current(4),
            Shanghai,
            None,
        ),
        "the acknowledged delta tail must extend the exact current cutoff"
    );
    assert!(
        summary_delta_gap_affects_selection(
            projection.as_ref(),
            std::slice::from_ref(&gap),
            std::slice::from_ref(&newer_delta),
            &SummaryWindow::Current(5),
            Shanghai,
            None,
        ),
        "a current limit reaching the first unproven rank must fail closed"
    );
    assert!(
        summary_delta_gap_affects_selection(
            projection.as_ref(),
            std::slice::from_ref(&gap),
            std::slice::from_ref(&newer_delta),
            &SummaryWindow::All,
            Shanghai,
            None,
        ),
        "an all-time gap must fail closed for both HTTP and Summary SSE"
    );
    assert!(
        !summary_delta_gap_affects_selection(
            projection.as_ref(),
            std::slice::from_ref(&gap),
            std::slice::from_ref(&newer_delta),
            &SummaryWindow::Current(50),
            Shanghai,
            Some(7),
        ),
        "a gap for another account must not hide an independent account selection"
    );
}

#[tokio::test]
async fn summary_projection_generation_fence_renews_only_unchanged_projection() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let occurred_at = db_occurred_at_lower_bound(Utc::now());
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-generation-fence', ?1, 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .bind(occurred_at)
        .execute(&state.pool)
        .await
        .expect("seed summary projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");

    let published_revision = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("published projection")
        .revision();
    let mut stale = (*state
        .subscription_hub
        .summary_projection()
        .await
        .expect("published projection"))
    .clone();
    stale.refreshed_at = Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state.subscription_hub.store_summary_projection(stale).await;
    let stale_response = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(stale_response, Err(ApiError::Unavailable(_))));

    assert!(
        renew_summary_projection_freshness_if_generation_matches(state.as_ref())
            .await
            .expect("read unchanged generation fence"),
        "matching durable source boundaries must renew the published projection"
    );
    let renewed = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("renewed projection");
    assert_eq!(
        renewed.revision, published_revision,
        "renewal must not rebuild or swap the snapshot"
    );
    state.pool.close().await;

    for window in ["current", "1d", "7d", "30d", "today"] {
        let Json(response) = fetch_summary(
            State(state.clone()),
            Query(SummaryQuery {
                window: Some(window.to_string()),
                limit: Some(50),
                time_zone: Some("Asia/Shanghai".to_string()),
                upstream_account_id: None,
            }),
        )
        .await
        .expect("renewed projection must remain a memory-only exact response");
        assert_eq!(response.total_count, 1, "{window} exact count");
        assert_eq!(response.total_tokens, 17, "{window} exact tokens");
    }
}

#[tokio::test]
async fn summary_projection_coverage_revision_invalidates_live_freshness_lease() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-coverage-renewal', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("seed summary projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");

    let mut stale = (*state
        .subscription_hub
        .summary_projection()
        .await
        .expect("published projection"))
    .clone();
    stale.refreshed_at = Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state.subscription_hub.store_summary_projection(stale).await;
    sqlx::query("UPDATE summary_coverage_revision SET revision = revision + 1 WHERE id = 1")
        .execute(&state.pool)
        .await
        .expect("advance coverage revision");

    assert!(
        !renew_summary_projection_freshness_if_generation_matches(state.as_ref())
            .await
            .expect("read changed coverage fence"),
        "coverage changes must not claim the full generation is unchanged"
    );
    assert!(
        !renew_summary_projection_freshness_if_live_tail_matches(state.as_ref())
            .await
            .expect("read changed coverage fence"),
        "coverage-only changes must not renew a projection with stale historical authority"
    );
}

#[tokio::test]
async fn summary_projection_coverage_change_revokes_stale_all_time_without_blocking_recent() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-coverage-revoke-all', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("seed summary projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish exact bootstrap projection");
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    refresh_summary_snapshots(state.as_ref())
        .await
        .expect("publish exact all-time projection");
    let Json(before) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("baseline all-time projection is exact");
    assert_eq!(before.total_count, 1);

    sqlx::query("UPDATE summary_coverage_revision SET revision = revision + 1 WHERE id = 1")
        .execute(&state.pool)
        .await
        .expect("advance historical coverage before build");
    refresh_summary_snapshots_with_mode(state.as_ref(), SummaryProjectionBuildMode::RollingDelta)
        .await
        .expect("coverage-only refresh must defer to historical supervisor");
    state.pool.close().await;

    let all_time = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(
        matches!(all_time, Err(ApiError::Unavailable(_))),
        "coverage revocation must not serve the old all-time aggregate"
    );

    let Json(current) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("unrelated recent selection remains exact");
    assert_eq!(current.total_count, 1);
    assert_eq!(current.total_tokens, 17);
}

#[tokio::test]
async fn summary_projection_generation_fence_rejects_changed_live_source() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-generation-fence-before', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("seed initial summary projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");

    let mut stale = (*state
        .subscription_hub
        .summary_projection()
        .await
        .expect("published projection"))
    .clone();
    stale.refreshed_at = Some(Instant::now() - SUMMARY_SNAPSHOT_MAX_STALE - Duration::from_secs(1));
    state.subscription_hub.store_summary_projection(stale).await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-generation-fence-after', datetime('now'), 'proxy', 'success', 19, 1.5, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("advance durable live source");

    assert!(
        !renew_summary_projection_freshness_if_generation_matches(state.as_ref())
            .await
            .expect("read changed generation fence"),
        "a changed durable source must rebuild instead of extending stale freshness"
    );
    let stale_response = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await;
    assert!(matches!(stale_response, Err(ApiError::Unavailable(_))));
    state.pool.close().await;
}

#[tokio::test]
async fn summary_all_time_coverage_page_requires_materialized_rollup_keys() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    let start = Utc::now() - ChronoDuration::hours(3);
    let end = start + ChronoDuration::hours(1);
    let archive_path = "/tmp/summary-coverage-proof-missing-rollup.sqlite.gz";
    sqlx::query(
            "INSERT INTO archive_batches \
             (dataset, month_key, file_path, sha256, row_count, status, coverage_start_at, coverage_end_at, \
              historical_rollups_materialized_at, upstream_activity_manifest_refreshed_at, created_at) \
             VALUES ('codex_invocations', '2026-01', ?1, 'summary-coverage-proof-missing-rollup', 1, \
                     'completed', ?2, ?3, datetime('now'), datetime('now'), datetime('now'))",
        )
        .bind(archive_path)
        .bind(db_occurred_at_lower_bound(start))
        .bind(db_occurred_at_upper_bound(end))
        .execute(&state.pool)
        .await
        .expect("insert materialized archive manifest");
    let archive_id: i64 = sqlx::query_scalar("SELECT id FROM archive_batches WHERE file_path = ?1")
        .bind(archive_path)
        .fetch_one(&state.pool)
        .await
        .expect("load archive manifest id");
    sqlx::query(
        "INSERT INTO archive_batch_upstream_activity \
             (archive_batch_id, account_id, last_activity_at) VALUES (?1, 42, ?2)",
    )
    .bind(archive_id)
    .bind(db_occurred_at_lower_bound(start))
    .execute(&state.pool)
    .await
    .expect("insert account manifest proof");
    for target in [
        HOURLY_ROLLUP_TARGET_INVOCATIONS,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_STATS_HOURLY,
        HOURLY_ROLLUP_TARGET_UPSTREAM_ACCOUNT_USAGE_BREAKDOWN,
    ] {
        sqlx::query(
            "INSERT INTO hourly_rollup_archive_replay \
                 (target, dataset, file_path, archive_sha256) VALUES (?1, 'codex_invocations', ?2, \
                 'summary-coverage-proof-missing-rollup')",
        )
        .bind(target)
        .bind(archive_path)
        .execute(&state.pool)
        .await
        .expect("insert replay proof");
    }

    let page = SummaryProjectionBoundaryManifestPage {
        archives: vec![
            crate::stats::ArchiveBatchPathRow::with_coverage_and_historical_rollups(
                archive_path,
                Some(db_occurred_at_lower_bound(start)),
                Some(db_occurred_at_upper_bound(end)),
                Some("now".to_string()),
            ),
        ],
        account_manifest_refreshed_paths: HashSet::from([archive_path.to_string()]),
        next_after_id: None,
    };
    let coverage = summary_all_time_coverage_page_is_exact(&state.pool, &page)
        .await
        .expect("evaluate materialized coverage proof");
    assert_eq!(coverage, (false, false));
}

#[tokio::test]
async fn summary_projection_all_time_finalization_uses_its_independent_stage_budget() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-all-time-stage-budget', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("seed bootstrap projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish rolling bootstrap before all-time finalization");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create finalization interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
    );
    let state_for_finalization = state.clone();
    let finalization = tokio::spawn(async move {
        refresh_summary_snapshots_with_mode(
            state_for_finalization.as_ref(),
            SummaryProjectionBuildMode::AllTime,
        )
        .await
    });

    tokio::time::timeout(Duration::from_secs(2), interleave.wait_for_writer())
        .await
        .expect("all-time build reaches its isolated staging point");
    // This intentionally exceeds both the former 8-second all-time deadline and the
    // 15-second serving freshness budget. The all-time build must keep the already-exact
    // rolling snapshot available while its independent reconciliation remains in flight.
    tokio::time::sleep(SUMMARY_SNAPSHOT_MAX_STALE + Duration::from_secs(1)).await;
    let Json(rolling) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("all-time reconciliation must not stale an already-published rolling response");
    assert_eq!(rolling.total_count, 1);
    assert_eq!(rolling.total_tokens, 17);
    interleave.resume_build();
    let finalized = tokio::time::timeout(Duration::from_secs(5), finalization)
        .await
        .expect("all-time finalization completes inside its independent budget");
    clear_summary_projection_test_interleave();
    finalized
        .expect("join all-time finalization")
        .expect("publish exact all-time projection");
    state.pool.close().await;

    let Json(response) = fetch_summary(
        State(state),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("finalized all-time response remains memory-only after SQLite closes");
    assert_eq!(response.total_count, 1);
    assert_eq!(response.total_tokens, 17);
}

#[tokio::test]
async fn summary_projection_generic_build_does_not_overwrite_newer_projection() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-generic-cas-base', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("seed generic projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish base projection");
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create generic build interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
    );
    let state_for_build = state.clone();
    let build = tokio::spawn(async move {
        hydrate_summary_snapshots_with_deadline(
            state_for_build.as_ref(),
            SUMMARY_PROJECTION_STARTUP_BUILD_DEADLINE,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), interleave.wait_for_writer())
        .await
        .expect("generic build reaches publication fence");

    let base = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("base projection remains published while build is paused");
    let replacement_revision = base.revision().saturating_add(1);
    state
        .subscription_hub
        .store_summary_projection(Arc::unwrap_or_clone(base).with_revision(replacement_revision))
        .await;
    interleave.resume_build();
    let result = tokio::time::timeout(Duration::from_secs(5), build)
        .await
        .expect("generic build completes after replacement")
        .expect("join generic build");
    clear_summary_projection_test_interleave();
    result.expect("stale generic build is discarded without error");

    let published = state
        .subscription_hub
        .summary_projection()
        .await
        .expect("replacement projection remains published");
    assert_eq!(published.revision(), replacement_revision);
}

#[tokio::test]
async fn summary_projection_all_time_generation_change_preserves_coverage_and_replays_tail() {
    let state = crate::tests::test_state_with_openai_base(
        url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
    )
    .await;
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-all-time-generation-before', datetime('now'), 'proxy', 'success', 17, 1.25, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("seed bootstrap projection");
    hydrate_summary_snapshots(state.as_ref())
        .await
        .expect("publish bootstrap projection");
    state
        .subscription_hub
        .note_summary_http_interest(true)
        .await;
    sqlx::query("CREATE TABLE summary_projection_test_interleave_gate (id INTEGER PRIMARY KEY)")
        .execute(&state.pool)
        .await
        .expect("create finalization interleave gate");
    let interleave = install_summary_projection_test_interleave_at(
        SummaryProjectionTestInterleaveStage::BeforeProjectionPublication,
    );
    let refresh_state = state.clone();
    let refresh =
        tokio::spawn(async move { refresh_summary_snapshots(refresh_state.as_ref()).await });

    tokio::time::timeout(Duration::from_secs(2), interleave.wait_for_writer())
        .await
        .expect("all-time reconciliation reaches its staging point");
    sqlx::query(
            "INSERT INTO codex_invocations \
             (invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response, detail_level) \
             VALUES ('summary-all-time-generation-after', datetime('now'), 'proxy', 'success', 23, 2.5, '{}', '', 'full')",
        )
        .execute(&state.pool)
        .await
        .expect("mutate the durable generation while all-time reconciliation is paused");

    let new_row = sqlx::query_as::<_, ApiInvocation>(
        "SELECT * FROM codex_invocations WHERE invoke_id = 'summary-all-time-generation-after'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load the committed terminal for the bounded tail overlay");
    let mut delta = persisted_dashboard_activity_terminal_delta(&new_row);
    delta.terminal_sequence = 2;
    delta.persisted_row_id = Some(new_row.id);
    state
        .subscription_hub
        .acknowledge_replayed_summary_delta(delta)
        .await;
    interleave.resume_build();

    let result = tokio::time::timeout(
        SUMMARY_PROJECTION_ALL_TIME_FINALIZATION_DEADLINE + Duration::from_secs(2),
        refresh,
    )
    .await
    .expect("coverage-stable all-time reconciliation must finish without cancellation");
    clear_summary_projection_test_interleave();
    result
        .expect("join refresh task")
        .expect("publish all-time projection while retaining the tail overlay");

    state.pool.close().await;
    let Json(current) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("current".to_string()),
            limit: Some(50),
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("current must remain exact and memory-only after all-time cancellation");
    assert_eq!(current.total_count, 2);
    assert_eq!(current.total_tokens, 40);

    let Json(all) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("all".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: None,
        }),
    )
    .await
    .expect("all-time finalization must include the concurrent terminal exactly once");
    assert_eq!(all.total_count, 2);
    assert_eq!(all.total_tokens, 40);
}
