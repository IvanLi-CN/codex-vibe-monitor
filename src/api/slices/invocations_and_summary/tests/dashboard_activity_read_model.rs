#[cfg(test)]
mod dashboard_activity_read_model_tests {
    use super::*;

    #[tokio::test]
    async fn persisted_dashboard_baseline_suppresses_stale_retry_ttft() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, status TEXT, payload TEXT, source TEXT, first_token_ms REAL, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, failure_class TEXT, failure_kind TEXT, error_message TEXT)",
        )
        .execute(&pool)
        .await
        .expect("create invocations table");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, attempt_index INTEGER NOT NULL, upstream_account_id INTEGER, status TEXT, phase TEXT, stream_latency_ms REAL, first_byte_latency_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("create attempts table");
        sqlx::query(
            "CREATE TABLE invocation_in_progress_live (invocation_id INTEGER NOT NULL, source TEXT, prompt_cache_key TEXT, upstream_ttfb_ms REAL)",
        )
        .execute(&pool)
        .await
        .expect("create live table");
        sqlx::query(
            "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, payload, source, first_token_ms, failure_class, error_message) VALUES (1, 'prior', '2026-08-03 08:00:00', 'failed', '{\"promptCacheKey\":\"conversation\",\"upstreamAccountId\":7}', 'proxy', NULL, 'service_failure', 'upstream failed'), (2, 'retried', '2026-08-03 08:00:10', 'running', '{\"promptCacheKey\":\"conversation\",\"upstreamAccountId\":7}', 'proxy', 720.0, NULL, NULL)",
        )
        .execute(&pool)
        .await
        .expect("insert invocation");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, attempt_index, upstream_account_id, status, phase, stream_latency_ms, first_byte_latency_ms) VALUES (1, 'retried', '2026-08-03 08:00:10', 1, 7, 'success', 'streaming_response', 400.0, 120.0), (2, 'retried', '2026-08-03 08:00:10', 2, 7, 'running', 'waiting_first_byte', NULL, NULL)",
        )
        .execute(&pool)
        .await
        .expect("insert attempts");
        sqlx::query(
            "INSERT INTO invocation_in_progress_live (invocation_id, source, prompt_cache_key, upstream_ttfb_ms) VALUES (2, 'proxy', 'conversation', 180.0)",
        )
        .execute(&pool)
        .await
        .expect("insert live row");

        let baseline =
            query_dashboard_runtime_projection_baseline(&pool, InvocationSourceScope::All)
                .await
                .expect("query persisted dashboard baseline");

        assert_eq!(baseline.records.len(), 1);
        assert!(baseline.records[0].is_retry);
        assert_eq!(
            baseline.records[0].live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_REQUESTING)
        );
    }

    async fn account_activity_aggregate_fixture() -> sqlx::SqlitePool {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (\
                id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, \
                status TEXT, total_tokens INTEGER, cost REAL, cache_input_tokens INTEGER, \
                payload TEXT, error_message TEXT, failure_kind TEXT, failure_class TEXT, \
                is_actionable INTEGER, t_req_read_ms REAL, t_req_parse_ms REAL, \
                t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, first_token_ms REAL, \
                t_upstream_stream_ms REAL, t_total_ms REAL, source TEXT\
             )",
        )
        .execute(&pool)
        .await
        .expect("create invocations table");
        sqlx::query(
            "CREATE TABLE pool_upstream_request_attempts (\
                id INTEGER PRIMARY KEY, invoke_id TEXT NOT NULL, occurred_at TEXT NOT NULL, \
                attempt_index INTEGER NOT NULL, status TEXT, phase TEXT, \
                stream_latency_ms REAL, first_byte_latency_ms REAL, upstream_account_id INTEGER\
             )",
        )
        .execute(&pool)
        .await
        .expect("create attempts table");
        sqlx::query(
            "CREATE TABLE prompt_cache_rollup_hourly (\
                prompt_cache_key TEXT NOT NULL, source TEXT, first_seen_at TEXT\
             )",
        )
        .execute(&pool)
        .await
        .expect("create prompt cache rollup table");
        sqlx::query(
            "CREATE TABLE prompt_cache_working_set_live (\
                prompt_cache_key TEXT NOT NULL, created_at TEXT, proxy_created_at TEXT\
             )",
        )
        .execute(&pool)
        .await
        .expect("create prompt cache working set table");
        sqlx::query(
            "INSERT INTO codex_invocations (\
                id, invoke_id, occurred_at, status, total_tokens, cost, cache_input_tokens, \
                payload, failure_class, first_token_ms, t_upstream_stream_ms, source\
             ) VALUES \
                (1, 'valid-final', '2026-08-03 08:00:00', 'success', 10, 0.01, 0, \
                 '{\"promptCacheKey\":\"valid\",\"upstreamAccountId\":7}', 'none', 720.0, 500.0, 'proxy'), \
                (2, 'stale-final', '2026-08-03 08:01:00', 'failed', 10, 0.01, 0, \
                 '{\"promptCacheKey\":\"stale\",\"upstreamAccountId\":7}', 'service_failure', 900.0, 800.0, 'proxy')",
        )
        .execute(&pool)
        .await
        .expect("insert invocations");
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (\
                id, invoke_id, occurred_at, attempt_index, status, phase, \
                stream_latency_ms, first_byte_latency_ms\
             ) VALUES \
                (1, 'valid-final', '2026-08-03 08:00:00', 1, 'failed', 'completed', 300.0, 100.0), \
                (2, 'valid-final', '2026-08-03 08:00:00', 2, 'success', 'completed', 500.0, 120.0), \
                (3, 'stale-final', '2026-08-03 08:01:00', 1, 'failed', 'completed', 800.0, 100.0), \
                (4, 'stale-final', '2026-08-03 08:01:00', 2, 'failed', 'completed', NULL, NULL)",
        )
        .execute(&pool)
        .await
        .expect("insert attempts");
        pool
    }

    #[tokio::test]
    async fn account_activity_aggregate_uses_only_final_retry_timing() {
        let pool = account_activity_aggregate_fixture().await;

        let rows = query_live_upstream_account_activity_aggregate_rows(
            &pool,
            InvocationSourceScope::All,
            ExactUtcRange {
                start: "2026-08-03T00:00:00Z".parse().expect("range start"),
                end: "2026-08-04T00:00:00Z".parse().expect("range end"),
            },
            true,
            DashboardActivityExcludedInvocationIdsFilter::None,
        )
        .await
        .expect("query account activity aggregate");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].upstream_account_id, Some(7));
        assert_eq!(rows[0].first_token_sample_count, 1);
        assert_eq!(rows[0].first_token_sum_ms, 720.0);

        let summary = query_invocation_network_summary(
            &pool,
            &InvocationRecordsFilters::default(),
            InvocationSourceScope::All,
            2,
        )
        .await
        .expect("query network summary");
        assert_eq!(summary.avg_first_token_ms, Some(720.0));
        assert_eq!(summary.p95_first_token_ms, Some(720.0));
        assert_eq!(summary.avg_response_duration_ms, Some(500.0));
        assert_eq!(summary.p95_response_duration_ms, Some(500.0));
    }

    #[tokio::test]
    async fn dashboard_activity_persisted_terminal_lookup_accepts_multiple_keys() {
        use sqlx::sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");
        sqlx::query(
            "CREATE TABLE codex_invocations (\
                id INTEGER PRIMARY KEY, \
                invoke_id TEXT NOT NULL, \
                occurred_at TEXT NOT NULL, \
                status TEXT NOT NULL, \
                source TEXT NOT NULL\
             )",
        )
        .execute(&pool)
        .await
        .expect("create invocations table");
        for (id, invoke_id, occurred_at) in [
            (1_i64, "terminal-a", "2026-07-28 10:00:00"),
            (2_i64, "terminal-b", "2026-07-28 10:01:00"),
        ] {
            sqlx::query(
                "INSERT INTO codex_invocations (id, invoke_id, occurred_at, status, source) \
                 VALUES (?1, ?2, ?3, 'success', 'proxy')",
            )
            .bind(id)
            .bind(invoke_id)
            .bind(occurred_at)
            .execute(&pool)
            .await
            .expect("insert terminal invocation");
        }

        let (cursor, persisted) = query_live_dashboard_activity_reconcile_probe(
            &pool,
            InvocationSourceScope::All,
            &[
                ("terminal-a".to_string(), "2026-07-28 10:00:00".to_string()),
                ("terminal-b".to_string(), "2026-07-28 10:01:00".to_string()),
            ],
        )
        .await
        .expect("look up multiple persisted terminal records");

        assert_eq!(cursor, 2);
        assert_eq!(persisted.len(), 2);
        assert!(persisted.values().all(|row_id| *row_id <= cursor));
        assert_eq!(
            persisted.get(&("terminal-a".to_string(), "2026-07-28 10:00:00".to_string())),
            Some(&1)
        );
        assert_eq!(
            persisted.get(&("terminal-b".to_string(), "2026-07-28 10:01:00".to_string())),
            Some(&2)
        );
    }

    #[test]
    fn open_range_snapshots_share_the_sixty_second_reconcile_cadence() {
        assert_eq!(
            dashboard_activity_snapshot_cache_ttl("1d"),
            Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS)
        );
        assert_eq!(
            dashboard_activity_snapshot_cache_ttl("7d"),
            Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS)
        );
        assert_eq!(
            dashboard_activity_snapshot_cache_ttl("today"),
            Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS)
        );
    }

    #[test]
    fn terminal_delta_updates_cached_summary_without_a_database_rebuild() {
        let mut snapshot = DashboardActivitySnapshot::test_stub("7d");
        snapshot.summary.stats.usage_breakdown = Some(UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
            models: Vec::new(),
        });
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 0;
        record.first_token_ms = Some(650.0);

        apply_dashboard_activity_terminal_delta(&mut snapshot, &record);

        assert_eq!(snapshot.summary.stats.total_count, 1);
        assert_eq!(snapshot.summary.stats.success_count, 1);
        assert_eq!(snapshot.summary.stats.total_tokens, 1_200);
        assert!((snapshot.summary.stats.total_cost - 0.0099).abs() < f64::EPSILON);
        assert_eq!(
            snapshot
                .summary
                .stats
                .usage_breakdown
                .as_ref()
                .expect("usage breakdown")
                .output_tokens,
            200
        );
        let account = snapshot.accounts.first().expect("terminal account");
        assert_eq!(account.upstream_account_id, Some(17));
        assert_eq!(account.request_count, 1);
        assert_eq!(account.first_byte_avg_ms, Some(548.0));
        assert_eq!(account.first_response_byte_total_avg_ms, Some(548.0));
        assert_eq!(account.first_token_avg_ms, Some(650.0));
        assert_eq!(account.avg_total_ms, Some(3_450.0));
        assert!(
            (account.cache_hit_rate.expect("cache hit rate") - (400.0 / 1_200.0)).abs()
                < f64::EPSILON
        );
        let summary_performance = &snapshot.summary.model_performance.total;
        assert_eq!(
            summary_performance.cumulative_usage_duration_ms,
            Some(3_450.0)
        );
        assert_eq!(summary_performance.avg_response_ms, Some(2_800.0));
        assert_eq!(
            summary_performance.avg_first_response_byte_total_ms,
            Some(548.0)
        );
        assert_eq!(
            account.model_performance.total.cumulative_usage_duration_ms,
            Some(3_450.0)
        );
        assert_eq!(account.model_performance.models.len(), 1);
    }

    #[test]
    fn terminal_delta_merges_account_latency_with_the_cached_baseline() {
        let mut snapshot = DashboardActivitySnapshot::test_stub("7d");
        let mut first = invocation_cost_audit_tests::sample_invocation(Some(25));
        first.id = 0;
        first.first_token_ms = Some(650.0);
        apply_dashboard_activity_terminal_delta(&mut snapshot, &first);

        let mut second = first.clone();
        second.invoke_id = "invocation-cost-audit-second".to_string();
        second.occurred_at = "2026-07-20 10:25:10".to_string();
        second.t_upstream_ttfb_ms = Some(430.0);
        second.first_token_ms = Some(750.0);
        second.t_total_ms = Some(4_450.0);
        apply_dashboard_activity_terminal_delta(&mut snapshot, &second);

        let account = snapshot.accounts.first().expect("terminal account");
        assert_eq!(account.request_count, 2);
        assert_eq!(account.first_byte_avg_ms, Some(598.0));
        assert_eq!(account.first_response_byte_total_avg_ms, Some(598.0));
        assert_eq!(account.first_token_avg_ms, Some(700.0));
        assert_eq!(account.avg_total_ms, Some(3_950.0));
    }

    #[test]
    fn expiry_delta_reverses_terminal_totals_without_a_database_build() {
        let mut snapshot = DashboardActivitySnapshot::test_stub("7d");
        snapshot.summary.stats.usage_breakdown = Some(UsageBreakdownResponse {
            cache_write_tokens: 0,
            cache_read_tokens: 0,
            output_tokens: 0,
            costs: None,
            models: Vec::new(),
        });
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let delta = dashboard_activity_terminal_delta(&record);
        apply_dashboard_activity_compact_terminal_delta(&mut snapshot, &delta);
        subtract_dashboard_activity_compact_terminal_delta(&mut snapshot, &delta);

        assert_eq!(snapshot.summary.stats.total_count, 0);
        assert_eq!(snapshot.summary.stats.total_tokens, 0);
        assert_eq!(snapshot.summary.stats.total_cost, 0.0);
        assert!(snapshot.accounts.is_empty());
        assert_eq!(
            snapshot
                .summary
                .stats
                .usage_breakdown
                .as_ref()
                .expect("usage breakdown")
                .output_tokens,
            0
        );
    }

    #[test]
    fn expiry_restores_the_latest_retained_invocation_for_the_account() {
        let mut snapshot = DashboardActivitySnapshot::test_stub("7d");
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let expired = dashboard_activity_terminal_delta(&record);
        apply_dashboard_activity_compact_terminal_delta(&mut snapshot, &expired);

        let mut retained = expired.clone();
        retained.invoke_id = "retained-latest".to_string();
        retained.occurred_at = "2026-07-20 10:30:00".to_string();
        apply_dashboard_activity_compact_terminal_delta(&mut snapshot, &retained);
        snapshot
            .accounts
            .first_mut()
            .expect("account")
            .last_invocation_at = Some(expired.occurred_at.clone());

        subtract_dashboard_activity_compact_terminal_delta(&mut snapshot, &expired);
        restore_dashboard_activity_last_invocation_after_expiry(
            &mut snapshot,
            &expired,
            &VecDeque::from([retained.clone()]),
        );

        assert_eq!(
            snapshot
                .accounts
                .first()
                .and_then(|account| account.last_invocation_at.as_deref()),
            Some(retained.occurred_at.as_str())
        );
    }

    #[test]
    fn rolling_cache_reuse_stops_at_captured_expiry_horizon() {
        let covered_until = Utc::now();
        let entry = DashboardActivitySnapshotCacheEntry {
            cached_at: Instant::now(),
            last_reconcile_attempted_at: Instant::now(),
            last_reconcile_failed: false,
            baseline_snapshot_cursor: 42,
            expiry_covered_until: Some(covered_until),
            expiry_terminal_deltas: VecDeque::new(),
            expiry_delta_estimated_bytes: 0,
            response: DashboardActivitySnapshot::test_stub("7d"),
        };

        assert!(dashboard_activity_entry_expiry_covers(
            &entry,
            ExactUtcRange {
                start: covered_until,
                end: covered_until + ChronoDuration::days(7),
            },
        ));
        assert!(!dashboard_activity_entry_expiry_covers(
            &entry,
            ExactUtcRange {
                start: covered_until + ChronoDuration::milliseconds(1),
                end: covered_until + ChronoDuration::days(7),
            },
        ));

        let uncovered_range = ExactUtcRange {
            start: covered_until + ChronoDuration::milliseconds(1),
            end: covered_until + ChronoDuration::days(7),
        };
        assert_eq!(
            dashboard_activity_snapshot_reuse_mode(
                &entry,
                uncovered_range,
                Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS),
            ),
            None
        );

        let mut failed_entry = entry;
        failed_entry.last_reconcile_failed = true;
        assert_eq!(
            dashboard_activity_snapshot_reuse_mode(
                &failed_entry,
                uncovered_range,
                Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS),
            ),
            Some(DashboardActivitySnapshotReuseMode::LastGoodReconcileBackoff)
        );

        let mut retry_due = failed_entry;
        retry_due.last_reconcile_attempted_at =
            Instant::now() - Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS + 1);
        assert_eq!(
            dashboard_activity_snapshot_reuse_mode(
                &retry_due,
                uncovered_range,
                Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS),
            ),
            None
        );
    }

    #[test]
    fn pressure_deferral_stops_after_five_minutes() {
        let mut entry = DashboardActivitySnapshotCacheEntry {
            cached_at: Instant::now(),
            last_reconcile_attempted_at: Instant::now(),
            last_reconcile_failed: false,
            baseline_snapshot_cursor: 42,
            expiry_covered_until: None,
            expiry_terminal_deltas: VecDeque::new(),
            expiry_delta_estimated_bytes: 0,
            response: DashboardActivitySnapshot::test_stub("7d"),
        };
        assert!(dashboard_activity_pressure_reconcile_deferred(&entry));

        entry.cached_at =
            Instant::now() - DASHBOARD_ACTIVITY_PRESSURE_RECONCILE_MAX_AGE - Duration::from_secs(1);
        assert!(!dashboard_activity_pressure_reconcile_deferred(&entry));
    }

    #[test]
    fn reconcile_failure_starts_last_good_backoff() {
        let covered_until = Utc::now() - ChronoDuration::seconds(1);
        let range = ExactUtcRange {
            start: Utc::now(),
            end: Utc::now() + ChronoDuration::days(7),
        };
        let mut entry = DashboardActivitySnapshotCacheEntry {
            cached_at: Instant::now()
                - Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS + 1),
            last_reconcile_attempted_at: Instant::now()
                - Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS + 1),
            last_reconcile_failed: false,
            baseline_snapshot_cursor: 42,
            expiry_covered_until: Some(covered_until),
            expiry_terminal_deltas: VecDeque::new(),
            expiry_delta_estimated_bytes: 0,
            response: DashboardActivitySnapshot::test_stub("7d"),
        };

        assert_eq!(
            dashboard_activity_snapshot_reuse_mode(
                &entry,
                range,
                Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS),
            ),
            None
        );
        mark_dashboard_activity_reconcile_failed(&mut entry);
        assert_eq!(
            dashboard_activity_snapshot_reuse_mode(
                &entry,
                range,
                Duration::from_secs(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS),
            ),
            Some(DashboardActivitySnapshotReuseMode::LastGoodReconcileBackoff)
        );
    }

    #[test]
    fn snapshot_cache_enforces_global_entry_and_expiry_budgets() {
        fn selection(index: usize) -> DashboardActivitySnapshotSelection {
            DashboardActivitySnapshotSelection {
                range: "7d".to_string(),
                range_anchor: "rolling".to_string(),
                time_zone: "UTC".to_string(),
                source_scope: "all".to_string(),
                recent_limit: index,
                include_accounts: true,
                include_recent: true,
            }
        }

        fn entry(age_secs: u64, expiry_bytes: usize) -> DashboardActivitySnapshotCacheEntry {
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now() - Duration::from_secs(age_secs),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: 42,
                expiry_covered_until: None,
                expiry_terminal_deltas: VecDeque::new(),
                expiry_delta_estimated_bytes: expiry_bytes,
                response: DashboardActivitySnapshot::test_stub("7d"),
            }
        }

        let mut cache = DashboardActivitySnapshotCacheState::default();
        for index in 0..=DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_ENTRIES {
            cache.entries.insert(
                selection(index),
                entry(
                    (DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_ENTRIES - index) as u64,
                    0,
                ),
            );
        }
        assert_eq!(
            prune_dashboard_activity_snapshot_entries(&mut cache, None),
            1
        );
        assert_eq!(
            cache.entries.len(),
            DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_ENTRIES
        );
        assert!(!cache.entries.contains_key(&selection(0)));

        cache.entries.clear();
        let protected = selection(2);
        cache.entries.insert(
            selection(1),
            entry(
                2,
                DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_EXPIRY_BYTES / 2 + 1,
            ),
        );
        cache.entries.insert(
            protected.clone(),
            entry(
                1,
                DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_MAX_EXPIRY_BYTES / 2 + 1,
            ),
        );
        assert_eq!(
            prune_dashboard_activity_snapshot_entries(&mut cache, Some(&protected)),
            1
        );
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.entries.contains_key(&protected));
    }

    #[test]
    fn rolling_cache_bypasses_ranges_with_an_archive_prefix() {
        let now = Utc::now();
        let range = ExactUtcRange {
            start: now - ChronoDuration::days(7),
            end: now,
        };

        assert!(!dashboard_activity_snapshot_cache_can_track_expiry(
            "7d",
            range,
            now - ChronoDuration::days(1),
        ));
        assert!(dashboard_activity_snapshot_cache_can_track_expiry(
            "7d",
            range,
            now - ChronoDuration::days(8),
        ));
        assert!(dashboard_activity_snapshot_cache_can_track_expiry(
            "today",
            range,
            now - ChronoDuration::days(1),
        ));
    }

    #[test]
    fn rolling_expiry_queue_orders_live_deltas_and_enforces_hard_limits() {
        let covered_until = Utc::now() + ChronoDuration::minutes(1);
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let mut later = dashboard_activity_terminal_delta(&record);
        later.occurred_at = format_naive(
            (covered_until - ChronoDuration::seconds(10))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let mut earlier = later.clone();
        earlier.invoke_id = "expiry-earlier".to_string();
        earlier.occurred_at = format_naive(
            (covered_until - ChronoDuration::seconds(20))
                .with_timezone(&Shanghai)
                .naive_local(),
        );
        let mut deltas = VecDeque::new();
        let mut estimated_bytes = 0usize;

        insert_dashboard_activity_expiry_delta(
            &mut deltas,
            &mut estimated_bytes,
            Some(covered_until),
            &later,
            parse_to_utc_datetime(&later.occurred_at).expect("later timestamp"),
        )
        .expect("insert later expiry delta");
        insert_dashboard_activity_expiry_delta(
            &mut deltas,
            &mut estimated_bytes,
            Some(covered_until),
            &earlier,
            parse_to_utc_datetime(&earlier.occurred_at).expect("earlier timestamp"),
        )
        .expect("insert earlier expiry delta");

        assert_eq!(
            deltas.front().map(|delta| delta.invoke_id.as_str()),
            Some("expiry-earlier")
        );
        assert_eq!(
            estimated_bytes,
            later.estimated_bytes + earlier.estimated_bytes
        );

        let mut full = std::iter::repeat_n(
            later.clone(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS,
        )
        .collect::<VecDeque<_>>();
        let mut full_bytes = 0usize;
        assert_eq!(
            insert_dashboard_activity_expiry_delta(
                &mut full,
                &mut full_bytes,
                Some(covered_until),
                &earlier,
                parse_to_utc_datetime(&earlier.occurred_at).expect("earlier timestamp"),
            ),
            Err("expiry_count_limit")
        );
    }

    #[test]
    fn terminal_delta_model_performance_matches_sql_qualification() {
        let mut accumulator = ModelPerformanceAccumulator::default();
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 0;
        record.cost = None;
        record.first_token_ms = Some(91.0);

        accumulator.add_terminal_record(&record);

        assert_eq!(accumulator.total_tokens, 0);
        assert_eq!(accumulator.stream_output_tokens, 0);
        assert_eq!(accumulator.stream_duration_ms, 0.0);
        assert_eq!(accumulator.response_sample_count, 1);
        assert_eq!(accumulator.response_sum_ms, 2_800.0);
        assert_eq!(accumulator.first_byte_sample_count, 0);
        assert_eq!(accumulator.first_token_sample_count, 1);
        assert_eq!(accumulator.first_token_sum_ms, 91.0);
        assert_eq!(accumulator.cumulative_usage_duration_sample_count, 0);
    }

    #[test]
    fn persisted_terminal_is_not_applied_again_to_covered_baseline() {
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 42;
        let entry = DashboardActivitySnapshotCacheEntry {
            cached_at: Instant::now(),
            last_reconcile_attempted_at: Instant::now(),
            last_reconcile_failed: false,
            baseline_snapshot_cursor: 42,
            expiry_covered_until: None,
            expiry_terminal_deltas: VecDeque::new(),
            expiry_delta_estimated_bytes: 0,
            response: DashboardActivitySnapshot::test_stub("7d"),
        };

        assert!(dashboard_activity_entry_includes_terminal(
            &entry,
            &dashboard_activity_terminal_delta(&record),
        ));
        record.id = 43;
        assert!(!dashboard_activity_entry_includes_terminal(
            &entry,
            &dashboard_activity_terminal_delta(&record),
        ));
    }

    #[test]
    fn acknowledged_terminal_below_baseline_cursor_is_not_replayed() {
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let mut delta = dashboard_activity_terminal_delta(&record);
        delta.persisted_row_id = Some(42);

        assert!(dashboard_activity_baseline_includes_pending_delta(
            &delta,
            42,
            &HashMap::new(),
        ));
        assert!(!dashboard_activity_baseline_includes_pending_delta(
            &delta,
            41,
            &HashMap::new(),
        ));
    }

    #[test]
    fn terminal_persisted_during_build_is_suppressed_by_the_post_build_probe() {
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let delta = dashboard_activity_terminal_delta(&record);
        let persisted_after_build = HashMap::from([(delta.key(), 43)]);

        assert!(dashboard_activity_baseline_includes_pending_delta(
            &delta,
            43,
            &persisted_after_build,
        ));
    }

    #[test]
    fn archive_prefix_snapshot_replays_only_unpersisted_terminal_deltas() {
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 0;
        record.occurred_at = (Utc::now() - ChronoDuration::hours(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let unpersisted = dashboard_activity_terminal_delta(&record);
        let mut persisted = unpersisted.clone();
        persisted.invoke_id = "already-persisted".to_string();
        persisted.persisted_row_id = Some(42);
        let occurred_at = parse_to_utc_datetime(&unpersisted.occurred_at)
            .expect("terminal timestamp should parse");
        let selection = build_dashboard_activity_snapshot_selection(
            "7d",
            ExactUtcRange {
                start: occurred_at - ChronoDuration::days(1),
                end: occurred_at + ChronoDuration::days(1),
            },
            chrono_tz::UTC,
            InvocationSourceScope::All,
            4,
            true,
            true,
        );
        let mut snapshot = DashboardActivitySnapshot::test_stub("7d");

        let replayed = replay_dashboard_activity_pending_deltas_without_expiry(
            &mut snapshot,
            &selection,
            &VecDeque::from([unpersisted, persisted]),
            42,
            &HashMap::new(),
        );

        assert_eq!(replayed, 1);
        assert_eq!(snapshot.summary.stats.total_count, 1);
    }

    #[test]
    fn compact_terminal_delta_enforces_count_and_byte_hard_limits() {
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let delta = dashboard_activity_terminal_delta(&record);
        let mut read_model = DashboardActivityReadModel {
            pending_terminal_deltas: std::iter::repeat_n(
                delta.clone(),
                DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS,
            )
            .collect(),
            ..DashboardActivityReadModel::default()
        };
        assert_eq!(
            dashboard_activity_terminal_delta_hard_limit_reason(&read_model, &delta),
            Some("count_limit")
        );

        read_model.pending_terminal_deltas.clear();
        read_model.pending_delta_estimated_bytes =
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES - delta.estimated_bytes + 1;
        assert_eq!(
            dashboard_activity_terminal_delta_hard_limit_reason(&read_model, &delta),
            Some("byte_limit")
        );
    }

    #[tokio::test]
    async fn hard_limit_rejection_does_not_publish_a_terminal_delta() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 0;
        record.invoke_id = "rejected-terminal-delta".to_string();
        let pending = dashboard_activity_terminal_delta(&record);
        {
            let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
            cache.read_model.pending_terminal_deltas =
                std::iter::repeat_n(pending, DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS)
                    .collect();
        }

        let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &record).await;

        assert_eq!(outcome.hard_limit_reason, Some("count_limit"));
        assert!(outcome.terminal_delta.is_none());
    }

    #[tokio::test]
    async fn expiry_hard_limit_does_not_publish_a_terminal_delta() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 0;
        record.invoke_id = "rejected-expiry-terminal-delta".to_string();
        record.occurred_at = (Utc::now() - ChronoDuration::hours(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        let occurred_at = parse_to_utc_datetime(&record.occurred_at).expect("record timestamp");
        let selection = build_dashboard_activity_snapshot_selection(
            "7d",
            ExactUtcRange {
                start: occurred_at - ChronoDuration::days(1),
                end: occurred_at + ChronoDuration::days(6),
            },
            chrono_tz::UTC,
            InvocationSourceScope::All,
            4,
            true,
            true,
        );
        let pending = dashboard_activity_terminal_delta(&record);
        {
            let mut cache = state.dashboard_activity_snapshot_cache.lock().await;
            cache.entries.insert(
                selection,
                DashboardActivitySnapshotCacheEntry {
                    cached_at: Instant::now(),
                    last_reconcile_attempted_at: Instant::now(),
                    last_reconcile_failed: false,
                    baseline_snapshot_cursor: 0,
                    expiry_covered_until: Some(occurred_at + ChronoDuration::hours(1)),
                    expiry_terminal_deltas: std::iter::repeat_n(
                        pending,
                        DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS,
                    )
                    .collect(),
                    expiry_delta_estimated_bytes: 0,
                    response: DashboardActivitySnapshot::test_stub("7d"),
                },
            );
        }

        let outcome = apply_dashboard_activity_terminal_record(state.as_ref(), &record).await;

        assert_eq!(outcome.hard_limit_reason, Some("expiry_count_limit"));
        assert!(outcome.terminal_delta.is_none());
    }

    #[tokio::test]
    async fn terminal_slice_without_an_owner_is_still_drainable() {
        let state = crate::tests::test_state_with_openai_base(
            url::Url::parse("http://127.0.0.1:9").expect("valid test URL"),
        )
        .await;
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 0;
        record.invoke_id = "subscriber-free-terminal-slice".to_string();
        record.occurred_at = (Utc::now() - ChronoDuration::minutes(1))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();

        let _registration =
            crate::terminal_projection::register_terminal_projection_before_enqueue(
                state.as_ref(),
                &record,
            )
            .await;

        assert_eq!(
            state
                .proxy_runtime_invocations
                .pending_terminal_slice_count(),
            1
        );
        let pending = state
            .proxy_runtime_invocations
            .pending_dashboard_publish_window()
            .expect("terminal publish window");
        assert_eq!(pending.slice, DashboardProjectionSlice::Terminal);
        let active = state
            .proxy_runtime_invocations
            .begin_dashboard_publish_window(pending)
            .expect("begin terminal publish window");
        assert!(
            state
                .proxy_runtime_invocations
                .capture_terminal_slice()
                .is_some()
        );
        state
            .proxy_runtime_invocations
            .complete_dashboard_publish_window(active);
        assert_eq!(
            state
                .proxy_runtime_invocations
                .pending_terminal_slice_count(),
            0
        );
    }

    #[test]
    fn persisted_recovery_delta_does_not_wait_for_a_writer_ack() {
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 42;
        let delta = dashboard_activity_terminal_delta(&record);

        assert!(!dashboard_activity_terminal_delta_needs_persistence_ack(
            &delta
        ));
    }

    #[test]
    fn hard_limit_dirty_state_waits_for_the_rejected_terminal_to_settle() {
        let mut read_model = DashboardActivityReadModel {
            hard_limit_reason: Some("count_limit"),
            hard_limit_sequence: Some(10),
            settled_terminal_sequence: 9,
            ..DashboardActivityReadModel::default()
        };

        clear_dashboard_activity_hard_limit_after_baseline(&mut read_model);
        assert_eq!(read_model.hard_limit_reason, Some("count_limit"));
        assert_eq!(read_model.hard_limit_sequence, Some(10));

        read_model.settled_terminal_sequence = 10;
        clear_dashboard_activity_hard_limit_after_baseline(&mut read_model);
        assert_eq!(read_model.hard_limit_reason, None);
        assert_eq!(read_model.hard_limit_sequence, None);
    }

    #[test]
    fn settled_hard_limit_forces_reconcile_before_cache_reuse() {
        let mut read_model = DashboardActivityReadModel {
            hard_limit_reason: Some("count_limit"),
            hard_limit_sequence: Some(10),
            settled_terminal_sequence: 9,
            ..DashboardActivityReadModel::default()
        };

        assert!(!dashboard_activity_read_model_requires_reconcile(
            &read_model
        ));
        read_model.settled_terminal_sequence = 10;
        assert!(dashboard_activity_read_model_requires_reconcile(
            &read_model
        ));
    }

    #[test]
    fn twenty_thousand_terminal_deltas_never_exceed_the_hard_limits() {
        let record = invocation_cost_audit_tests::sample_invocation(Some(25));
        let mut read_model = DashboardActivityReadModel::default();
        let mut rejected = 0usize;

        for index in 0..20_000 {
            let mut delta = dashboard_activity_terminal_delta(&record);
            delta.invoke_id = format!("terminal-{index}");
            if dashboard_activity_terminal_delta_hard_limit_reason(&read_model, &delta).is_some() {
                rejected += 1;
                continue;
            }
            read_model.pending_delta_estimated_bytes = read_model
                .pending_delta_estimated_bytes
                .saturating_add(delta.estimated_bytes);
            read_model.pending_terminal_deltas.push_back(delta);
        }

        assert_eq!(
            read_model.pending_terminal_deltas.len(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_TERMINALS
        );
        assert_eq!(rejected, 10_000);
        assert!(
            read_model.pending_delta_estimated_bytes
                <= DASHBOARD_ACTIVITY_READ_MODEL_MAX_PENDING_BYTES
        );
    }

    #[test]
    fn applied_terminal_keys_remain_bounded_on_hard_limit_rejection() {
        let mut read_model = DashboardActivityReadModel::default();
        for index in 0..=DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS {
            insert_dashboard_activity_applied_terminal_key(
                &mut read_model,
                (
                    format!("invoke-{index}"),
                    format!("2026-07-28 10:{:02}:00", index % 60),
                ),
            );
        }

        prune_dashboard_activity_applied_terminal_keys(&mut read_model);

        assert_eq!(
            read_model.applied_terminal_keys.len(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS
        );
        assert_eq!(
            read_model.applied_terminal_key_order.len(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS
        );

        let rolled_back_key = read_model
            .applied_terminal_key_order
            .front()
            .expect("oldest applied key")
            .0
            .clone();
        read_model.applied_terminal_keys.remove(&rolled_back_key);
        prune_dashboard_activity_applied_terminal_keys(&mut read_model);
        assert_eq!(
            read_model.applied_terminal_key_order.len(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS - 1
        );

        insert_dashboard_activity_applied_terminal_key(
            &mut read_model,
            (
                "steady-state".to_string(),
                "2026-07-28 11:00:00".to_string(),
            ),
        );
        prune_dashboard_activity_applied_terminal_keys(&mut read_model);
        assert_eq!(
            read_model.applied_terminal_keys.len(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS
        );
        assert_eq!(
            read_model.applied_terminal_key_order.len(),
            DASHBOARD_ACTIVITY_READ_MODEL_MAX_TERMINAL_KEYS
        );
    }

    #[test]
    fn acknowledged_delta_waits_for_every_warm_cursor_before_pruning() {
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 42;
        let delta = dashboard_activity_terminal_delta(&record);
        let mut cache = DashboardActivitySnapshotCacheState::default();
        cache.read_model.pending_delta_estimated_bytes = delta.estimated_bytes;
        cache.read_model.persisted_ack_pending_count = 1;
        cache.read_model.pending_terminal_deltas.push_back(delta);
        let selection = build_dashboard_activity_snapshot_selection(
            "7d",
            ExactUtcRange {
                start: Utc::now() - ChronoDuration::days(7),
                end: Utc::now(),
            },
            chrono_tz::UTC,
            InvocationSourceScope::All,
            4,
            true,
            true,
        );
        cache.entries.insert(
            selection,
            DashboardActivitySnapshotCacheEntry {
                cached_at: Instant::now(),
                last_reconcile_attempted_at: Instant::now(),
                last_reconcile_failed: false,
                baseline_snapshot_cursor: 41,
                expiry_covered_until: None,
                expiry_terminal_deltas: VecDeque::new(),
                expiry_delta_estimated_bytes: 0,
                response: DashboardActivitySnapshot::test_stub("7d"),
            },
        );

        prune_dashboard_activity_terminal_deltas(&mut cache);
        assert_eq!(cache.read_model.pending_terminal_deltas.len(), 1);
        cache
            .entries
            .values_mut()
            .next()
            .expect("warm selection")
            .baseline_snapshot_cursor = 42;
        prune_dashboard_activity_terminal_deltas(&mut cache);
        assert!(cache.read_model.pending_terminal_deltas.is_empty());
        assert_eq!(cache.read_model.delta_pruned_count, 1);
    }

    #[tokio::test]
    async fn repeated_ack_after_safe_prune_is_idempotent() {
        let mut record = invocation_cost_audit_tests::sample_invocation(Some(25));
        record.id = 42;
        let mut delta = dashboard_activity_terminal_delta(&record);
        delta.terminal_sequence = 1;
        let terminal_sequence = delta.terminal_sequence;
        let mut cache = DashboardActivitySnapshotCacheState::default();
        cache
            .read_model
            .applied_terminal_keys
            .insert(delta.key(), Instant::now());
        cache.read_model.pending_delta_estimated_bytes = delta.estimated_bytes;
        cache.read_model.pending_terminal_deltas.push_back(delta);
        let cache = Arc::new(tokio::sync::Mutex::new(cache));

        acknowledge_dashboard_activity_terminal_record(
            &cache,
            &record.invoke_id,
            &record.occurred_at,
            record.id,
            Some(terminal_sequence),
        )
        .await;
        acknowledge_dashboard_activity_terminal_record(
            &cache,
            &record.invoke_id,
            &record.occurred_at,
            record.id,
            Some(terminal_sequence),
        )
        .await;

        let cache = cache.lock().await;
        assert!(cache.read_model.pending_terminal_deltas.is_empty());
        assert_eq!(cache.read_model.sequence_gap_count, 0);
        assert_eq!(cache.read_model.hard_limit_reason, None);
    }

    #[test]
    fn out_of_order_terminal_ack_only_advances_the_contiguous_watermark() {
        let mut read_model = DashboardActivityReadModel::default();

        settle_dashboard_activity_terminal_sequence(&mut read_model, 2);
        assert_eq!(read_model.settled_terminal_sequence, 0);
        assert!(read_model.settled_terminal_sequences.contains(&2));

        settle_dashboard_activity_terminal_sequence(&mut read_model, 1);
        assert_eq!(read_model.settled_terminal_sequence, 2);
        assert!(read_model.settled_terminal_sequences.is_empty());
    }

    #[test]
    fn expiry_keeps_model_with_first_byte_samples() {
        let mut accumulator = ModelPerformanceAccumulator::default();
        let mut expiring = dashboard_activity_terminal_delta(
            &invocation_cost_audit_tests::sample_invocation(Some(25)),
        );
        expiring.t_upstream_stream_ms = Some(100.0);
        expiring.first_token_ms = None;
        expiring.t_total_ms = None;
        expiring.t_upstream_ttfb_ms = None;
        accumulator.add_terminal_delta(&expiring);

        let mut retained = expiring.clone();
        retained.invoke_id = "retained-first-byte".to_string();
        retained.total_tokens = 0;
        retained.output_tokens = 0;
        retained.t_upstream_stream_ms = None;
        retained.t_upstream_ttfb_ms = Some(250.0);
        accumulator.add_terminal_delta(&retained);
        accumulator.subtract_terminal_delta(&expiring);

        let group = UsageBreakdownGroupKey {
            model: retained.model,
            reasoning_effort: retained.reasoning_effort,
        };
        let retained_model = accumulator.models.get(&group).expect("first-byte model");
        assert_eq!(retained_model.first_byte_sample_count, 1);
    }
}
