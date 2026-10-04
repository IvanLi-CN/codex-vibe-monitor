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
async fn invocation_ranges_hot_issuance_survives_closed_database() {
    let pool = ranges_fixture().await;
    let manager =
        Arc::new(prompt_cache_conversations::invocation_ranges::InvocationRangeManager::default());
    let first = manager.allocate(&pool, Some("memory-only")).await.unwrap();
    let ceiling: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='memory-only'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(ceiling, 63);
    pool.close().await;
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
