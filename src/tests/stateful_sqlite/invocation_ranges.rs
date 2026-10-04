use super::*;

async fn ranges_fixture() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    ensure_schema(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn invocation_ranges_schema_marker_and_ddl_roll_back_together() {
    let pool = ranges_fixture().await;
    sqlx::query("DELETE FROM schema_refresh_migrations WHERE migration_name='invocation_range_ownership_v1'")
        .execute(&pool).await.unwrap();
    sqlx::query("DROP TABLE hourly_invoke_prefixes")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER fail_range_marker BEFORE INSERT ON schema_refresh_migrations WHEN NEW.migration_name='invocation_range_ownership_v1' BEGIN SELECT RAISE(ABORT,'interrupted upgrade'); END")
        .execute(&pool).await.unwrap();
    assert!(
        prompt_cache_conversations::invocation_ranges::ensure_schema(&pool)
            .await
            .is_err()
    );
    let installed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE name='hourly_invoke_prefixes'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(installed, 0);
    sqlx::query("DROP TRIGGER fail_range_marker")
        .execute(&pool)
        .await
        .unwrap();
    prompt_cache_conversations::invocation_ranges::ensure_schema(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO hourly_invoke_prefixes (utc_hour,prefix,last_invoke_sequence) VALUES (1,'ABCDEF',63)")
        .execute(&pool).await.unwrap();
    prompt_cache_conversations::invocation_ranges::ensure_schema(&pool)
        .await
        .unwrap();
    let ceiling: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM hourly_invoke_prefixes WHERE utc_hour=1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ceiling, 63);
}

#[tokio::test]
async fn invocation_ranges_statistics_do_not_own_reservation_ceiling() {
    let pool = ranges_fixture().await;
    sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invoke_sequence) VALUES ('ABCDEF','range-stats',63)")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response) VALUES ('ABCDEFABAA','2026-10-05T00:00:00Z','proxy','success','{\"promptCacheKey\":\"range-stats\"}','{}')")
        .execute(&pool).await.unwrap();
    refresh_prompt_cache_conversation_stats(&pool, &HashSet::from(["range-stats".to_string()]))
        .await
        .unwrap();
    let row: (i64, i64) = sqlx::query_as("SELECT last_invoke_sequence,request_count FROM prompt_cache_conversations WHERE prompt_cache_key='range-stats'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(row, (63, 1));
}

#[tokio::test]
async fn invocation_ranges_legacy_live_floor_survives_upgrade_and_statistics() {
    let pool = ranges_fixture().await;
    sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invoke_sequence) VALUES ('ABCDEF','legacy-floor',9)")
        .execute(&pool).await.unwrap();
    let retained = format!(
        "ABCDEF{}",
        encode_prompt_cache_conversation_sequence(100).unwrap()
    );
    sqlx::query("INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,payload,raw_response) VALUES (?1,'2026-10-05T00:00:00Z','proxy','success','{\"promptCacheKey\":\"legacy-floor\"}','{}')")
        .bind(&retained).execute(&pool).await.unwrap();
    prompt_cache_conversations::invocation_ranges::ensure_schema(&pool)
        .await
        .unwrap();
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let issued = manager.allocate(&pool, Some("legacy-floor")).await.unwrap();
    assert_eq!(
        issued,
        format!(
            "ABCDEF{}",
            encode_prompt_cache_conversation_sequence(101).unwrap()
        )
    );
    refresh_prompt_cache_conversation_stats(&pool, &HashSet::from(["legacy-floor".to_owned()]))
        .await
        .unwrap();
    let ceiling: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='legacy-floor'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(ceiling, 164);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT invoke_id FROM codex_invocations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        retained
    );
}

#[tokio::test]
async fn invocation_ranges_hot_issuance_survives_closed_database() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let first = manager.allocate(&pool, Some("memory-only")).await.unwrap();
    let hourly = manager.allocate(&pool, None).await.unwrap();
    let ceiling: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='memory-only'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(ceiling, 63);
    pool.close().await;
    let _blocker = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    for sequence in 1..=30 {
        let id = manager.allocate(&pool, Some("memory-only")).await.unwrap();
        assert_eq!(
            id,
            format!(
                "{}{}",
                &first[..6],
                encode_prompt_cache_conversation_sequence(sequence).unwrap()
            )
        );
        assert_eq!(
            manager.allocate(&pool, None).await.unwrap(),
            format!(
                "{}{}",
                &hourly[..6],
                encode_prompt_cache_conversation_sequence(sequence).unwrap()
            )
        );
    }
}

#[tokio::test]
async fn invocation_ranges_hour_reentry_skips_unpublished_reservations() {
    use prompt_cache_conversations::invocation_ranges::InvocationRangeManager;
    let pool = ranges_fixture().await;
    let first = Arc::new(InvocationRangeManager::default())
        .allocate(&pool, None)
        .await
        .unwrap();
    let second = Arc::new(InvocationRangeManager::default())
        .allocate(&pool, None)
        .await
        .unwrap();
    assert_eq!(&first[..6], &second[..6]);
    assert_eq!(&first[6..], "AAAA");
    assert_eq!(
        &second[6..],
        encode_prompt_cache_conversation_sequence(64).unwrap()
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM hourly_invoke_prefixes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn invocation_ranges_parallel_issuance_is_unique() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    manager
        .allocate(&pool, Some("concurrent-ranges"))
        .await
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..300 {
        let manager = manager.clone();
        let pool = pool.clone();
        tasks.spawn(async move {
            manager
                .allocate(&pool, Some("concurrent-ranges"))
                .await
                .unwrap()
        });
    }
    let mut ids = HashSet::new();
    while let Some(result) = tasks.join_next().await {
        assert!(ids.insert(result.unwrap()));
    }
    assert_eq!(ids.len(), 300);
    assert!(ids.iter().all(|id| id.len() == 10));
}

#[tokio::test]
async fn invocation_ranges_mixed_batch_respects_strict_thresholds() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    for (key, count) in [
        (Some("trigger"), 32),
        (Some("join"), 17),
        (Some("exclude"), 16),
        (None, 17),
    ] {
        for _ in 0..count {
            manager.allocate(&pool, key).await.unwrap();
        }
    }
    let ceilings: Vec<i64> = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations UNION ALL SELECT last_invoke_sequence FROM hourly_invoke_prefixes")
        .fetch_all(&pool).await.unwrap();
    assert_eq!(ceilings, [63, 63, 63, 63]);
    let blocker = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    manager.allocate(&pool, Some("trigger")).await.unwrap();
    drop(blocker);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let reserved: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM prompt_cache_conversations WHERE last_invoke_sequence=127",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if reserved == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let excluded: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='exclude'").fetch_one(&pool).await.unwrap();
    let hourly: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM hourly_invoke_prefixes")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(excluded, 63);
    assert_eq!(hourly, 127);
}

#[tokio::test]
async fn invocation_ranges_final_partial_segment_never_wraps() {
    let pool = ranges_fixture().await;
    sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invoke_sequence) VALUES ('ABCDEF','final-range',?1)")
        .bind(i64::from(PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY) - 21).execute(&pool).await.unwrap();
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    for sequence in (PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY - 20)
        ..PROMPT_CACHE_CONVERSATION_SEQUENCE_CAPACITY
    {
        let id = manager.allocate(&pool, Some("final-range")).await.unwrap();
        assert_eq!(
            id,
            format!(
                "ABCDEF{}",
                encode_prompt_cache_conversation_sequence(sequence).unwrap()
            )
        );
    }
    assert!(
        manager
            .allocate(&pool, Some("final-range"))
            .await
            .unwrap_err()
            .to_string()
            .contains("overflow")
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='final-range'").fetch_one(&pool).await.unwrap(), 923520);
}

#[tokio::test]
async fn invocation_ranges_failed_commit_cannot_publish() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let first = manager
        .allocate(&pool, Some("failed-refill"))
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_refill BEFORE UPDATE OF last_invoke_sequence ON prompt_cache_conversations WHEN NEW.last_invoke_sequence>63 BEGIN SELECT RAISE(ABORT,'refill unavailable'); END")
        .execute(&pool).await.unwrap();
    for sequence in 1..64 {
        let id = manager
            .allocate(&pool, Some("failed-refill"))
            .await
            .unwrap();
        assert_eq!(
            id,
            format!(
                "{}{}",
                &first[..6],
                encode_prompt_cache_conversation_sequence(sequence).unwrap()
            )
        );
    }
    assert!(
        manager
            .allocate(&pool, Some("failed-refill"))
            .await
            .is_err()
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='failed-refill'").fetch_one(&pool).await.unwrap(), 63);
}

#[tokio::test]
async fn invocation_ranges_activity_is_bounded_independent_and_memory_only() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    assert_eq!(manager.test_resize(0), (128, 0, 0));
    pool.close().await;
    for number in 0..4200 {
        let prefix = format!(
            "AA{}",
            encode_prompt_cache_conversation_sequence(number).unwrap()
        );
        manager.test_observe_activity(&prefix, i64::from(number));
    }
    assert_eq!(manager.test_resize(4200), (4096, 0, 4096));
    manager.test_observe_activity("ZZZZZZ", 48 * 3600 * 1000 + 4200);
    manager.test_observe_activity("ZZZZZZ", 1); // A delayed seed cannot overwrite live activity.
    assert_eq!(manager.test_resize(48 * 3600 * 1000 + 4201), (128, 0, 1));
    assert_eq!(manager.test_resize(96 * 3600 * 1000 + 4201), (128, 0, 0));
}

#[tokio::test]
async fn invocation_ranges_retirement_returns_only_unissued_tail() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let first = manager.allocate(&pool, Some("return-tail")).await.unwrap();
    manager.test_retire(&pool, "return-tail");
    let next = manager.allocate(&pool, Some("return-tail")).await.unwrap();
    assert_eq!(next, format!("{}AAAB", &first[..6]));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='return-tail'").fetch_one(&pool).await.unwrap(), 64);
    assert_eq!(manager.test_resize(Utc::now().timestamp_millis()).2, 1);
}

#[tokio::test]
async fn invocation_ranges_seed_uses_bounded_covering_index() {
    let pool = ranges_fixture().await;
    let rows = sqlx::query("EXPLAIN QUERY PLAN SELECT conversation_id,last_invocation_at FROM prompt_cache_conversations INDEXED BY idx_prompt_cache_conversations_last_invocation WHERE last_invocation_at>=?1 ORDER BY last_invocation_at DESC,conversation_id LIMIT 4096")
        .bind("2026-10-01 00:00:00").fetch_all(&pool).await.unwrap();
    let details = rows
        .iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>();
    assert!(
        details.iter().any(|detail| detail
            .contains("COVERING INDEX idx_prompt_cache_conversations_last_invocation")),
        "{details:?}"
    );
    assert!(
        details.iter().all(|detail| !detail.contains("TEMP B-TREE")
            && !detail.contains("SCAN prompt_cache_conversations")),
        "{details:?}"
    );
}

#[tokio::test]
async fn invocation_ranges_protected_cache_saturation_has_bounded_admission() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    for number in 0..4096 {
        let key = format!("protected-{number}");
        manager.retain_key(&key);
        // This prepares the protected working set, not the measured saturated
        // request. A cold admission may legitimately time out under parallel
        // profile load while its independent initialization keeps running.
        let mut initialized = false;
        for _ in 0..5 {
            match manager.allocate(&pool, Some(&key)).await {
                Ok(_) => {
                    initialized = true;
                    break;
                }
                Err(error) => assert!(error.to_string().contains("timed out"), "{error}"),
            }
        }
        assert!(initialized, "fixture owner {number} did not initialize");
    }
    assert_eq!(manager.test_resize(0).1, 4096);
    pool.close().await;
    let started = Instant::now();
    assert!(
        manager
            .allocate(&pool, Some("overflow-owner"))
            .await
            .unwrap_err()
            .to_string()
            .contains("admission timed out")
    );
    assert!(started.elapsed() < Duration::from_millis(200));
    assert!(manager.allocate(&pool, Some("protected-0")).await.is_ok());
}

#[tokio::test]
async fn invocation_ranges_late_lease_callback_keeps_namespace_until_drained() {
    let pool = ranges_fixture().await;
    let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let manager = cache.lock().await.identity_cache.range_manager.clone();
    let hour = Utc::now().timestamp().div_euclid(3600) - 1;
    let id = manager.test_allocate_hour(&pool, hour).await.unwrap();
    let guard = PromptCacheInvocationLeaseGuard::new(cache.clone(), &id);
    manager.reconcile_persistence(&id);
    manager.cleanup_hours(&pool, false).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let callback_blocker = cache.lock().await;
    drop(guard);
    manager.cleanup_hours(&pool, false).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    drop(callback_blocker);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            manager.cleanup_hours(&pool, false).await.unwrap();
            if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
                .fetch_one(&pool)
                .await
                .unwrap()
                == 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!prompt_cache_conversations::invocation_ranges::lifecycle::prefix_occupied(&id[..6]));
}

#[tokio::test]
async fn invocation_ranges_cold_admission_does_not_wait_for_unrelated_tail_return() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    for number in 0..128 {
        manager
            .allocate(&pool, Some(&format!("idle-{number}")))
            .await
            .unwrap();
    }
    let blocker = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    manager.test_retire(&pool, "idle-0");
    let allocation = {
        let manager = manager.clone();
        let pool = pool.clone();
        tokio::spawn(async move { manager.allocate(&pool, Some("new-owner")).await })
    };
    tokio::time::timeout(Duration::from_millis(80), async {
        while manager.occupancy() != 129 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("new owner reserves its slot before the unrelated return completes");
    drop(blocker);
    allocation.await.unwrap().unwrap();
}

#[tokio::test]
async fn invocation_ranges_busy_return_discards_tail_without_blocking_hot_owner() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let first = manager.allocate(&pool, Some("busy-return")).await.unwrap();
    manager
        .allocate(&pool, Some("unrelated-hot"))
        .await
        .unwrap();
    let blocker = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
        .acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P1Terminal)
        .await;
    manager.test_retire(&pool, "busy-return");
    let started = Instant::now();
    manager
        .allocate(&pool, Some("unrelated-hot"))
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_millis(100));
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='busy-return'").fetch_one(&pool).await.unwrap(), 63);
    drop(blocker);
    let next = manager.allocate(&pool, Some("busy-return")).await.unwrap();
    assert_eq!(
        next,
        format!(
            "{}{}",
            &first[..6],
            encode_prompt_cache_conversation_sequence(64).unwrap()
        )
    );
}

#[tokio::test]
async fn invocation_ranges_async_seed_merges_live_activity_without_preallocation() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let now = Utc::now();
    let old_time = format_naive(
        (now - chrono::Duration::hours(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    for number in 0..200 {
        let prefix = format!(
            "AA{}",
            encode_prompt_cache_conversation_sequence(number).unwrap()
        );
        sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invocation_at) VALUES (?1,?2,?3)")
            .bind(&prefix).bind(format!("seed-{number}")).bind(&old_time).execute(&pool).await.unwrap();
    }
    manager.test_observe_activity("AAAAAA", now.timestamp_millis());
    assert_eq!(manager.test_resize(now.timestamp_millis()), (128, 0, 1));
    let shutdown = CancellationToken::new();
    manager.start_sizing(&pool, shutdown.clone());
    tokio::time::timeout(Duration::from_secs(2), async {
        while manager.test_resize(now.timestamp_millis()).2 != 200 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        manager.test_activity_time("AAAAAA"),
        Some(now.timestamp_millis())
    );
    assert_eq!(manager.test_resize(now.timestamp_millis()), (200, 0, 200));
    let ceilings: Vec<i64> =
        sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(ceilings.iter().all(|ceiling| *ceiling == -1));
    shutdown.cancel();
    pool.close().await;
    assert_eq!(
        manager.test_resize(now.timestamp_millis() + 47 * 3600 * 1000),
        (128, 0, 1)
    );
}

#[tokio::test]
async fn invocation_ranges_pending_recovery_floor_and_namespace_release() {
    use prompt_cache_conversations::invocation_ranges::{
        InvocationRangeManager, lifecycle::PendingIdentityRegistry,
    };
    let pool = ranges_fixture().await;
    sqlx::query("INSERT INTO prompt_cache_conversations (conversation_id,prompt_cache_key,last_invoke_sequence) VALUES ('ABCDEF','pending-floor',5)").execute(&pool).await.unwrap();
    let pending = PendingIdentityRegistry::default();
    let pending_id = format!(
        "ABCDEF{}",
        encode_prompt_cache_conversation_sequence(100).unwrap()
    );
    pending.register(&pending_id, "2026-10-01 00:00:00", false);
    let manager = Arc::new(InvocationRangeManager::default());
    let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    cache.lock().await.identity_cache.range_manager = manager.clone();
    let id = manager
        .allocate(&pool, Some("pending-floor"))
        .await
        .unwrap();
    assert_eq!(
        id,
        format!(
            "ABCDEF{}",
            encode_prompt_cache_conversation_sequence(101).unwrap()
        )
    );
    sqlx::query("UPDATE prompt_cache_conversations SET last_invocation_at='2026-01-01 00:00:00',updated_at='2026-01-01 00:00:00'").execute(&pool).await.unwrap();
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(&pool, false, &cache)
            .await
            .unwrap(),
        0
    );
    pending.acknowledge(&pending_id, "2026-10-01 00:00:00", false);
    assert_eq!(
        cleanup_orphan_prompt_cache_conversations_with_cache(&pool, false, &cache)
            .await
            .unwrap(),
        1
    );
    let identity = create_prompt_cache_conversation_row_with_test_candidates(
        &pool,
        "reuse-after-release",
        &["ABCDEF"],
    )
    .await
    .unwrap();
    assert_eq!(identity.conversation_id, "ABCDEF");
}

#[tokio::test]
async fn invocation_ranges_hour_rollover_protects_active_pending_and_retained_ids() {
    use prompt_cache_conversations::invocation_ranges::{
        InvocationRangeManager, lifecycle::PendingIdentityRegistry,
    };
    let pool = ranges_fixture().await;
    let manager = Arc::new(InvocationRangeManager::default());
    let old_hour = Utc::now().timestamp().div_euclid(3600) - 1;
    let old_id = manager.test_allocate_hour(&pool, old_hour).await.unwrap();
    let current_id = manager.allocate(&pool, None).await.unwrap();
    assert_ne!(&old_id[..6], &current_id[..6]);
    manager.cleanup_hours(&pool, false).await.unwrap();
    let count = || {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM hourly_invoke_prefixes WHERE utc_hour=?1",
        )
        .bind(old_hour)
        .fetch_one(&pool)
    };
    assert_eq!(count().await.unwrap(), 1);
    let pending = PendingIdentityRegistry::default();
    pending.register(&old_id, "2026-10-01 00:00:00", false);
    manager.reconcile_persistence(&old_id);
    manager.cleanup_hours(&pool, false).await.unwrap();
    assert_eq!(count().await.unwrap(), 1);
    sqlx::query("INSERT INTO codex_invocations (invoke_id,occurred_at,source,status,raw_response) VALUES (?1,'2026-10-01 00:00:00','proxy','success','{}')").bind(&old_id).execute(&pool).await.unwrap();
    pending.acknowledge(&old_id, "2026-10-01 00:00:00", false);
    manager.cleanup_hours(&pool, false).await.unwrap();
    assert_eq!(count().await.unwrap(), 1);
    sqlx::query("DELETE FROM codex_invocations WHERE invoke_id=?1")
        .bind(&old_id)
        .execute(&pool)
        .await
        .unwrap();
    manager.cleanup_hours(&pool, false).await.unwrap();
    assert_eq!(count().await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(
        !prompt_cache_conversations::invocation_ranges::lifecycle::prefix_occupied(&old_id[..6])
    );
}

#[tokio::test]
async fn invocation_ranges_pending_raw_and_terminal_references_are_independent() {
    use prompt_cache_conversations::invocation_ranges::lifecycle::{
        PendingIdentityRegistry, pending_floor, pending_prefix,
    };
    let pending = PendingIdentityRegistry::default();
    pending.register("ABCDEFAAAA", "time", false);
    pending.register("ABCDEFAAAA", "time", false);
    pending.register("ABCDEFAAAA", "time", true);
    assert_eq!(pending_floor("ABCDEF"), 0);
    pending.acknowledge("ABCDEFAAAA", "time", false);
    assert!(pending_prefix("ABCDEF"));
    pending.acknowledge("ABCDEFAAAA", "time", true);
    assert!(!pending_prefix("ABCDEF"));
}
