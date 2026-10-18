use super::*;
use prompt_cache_conversations::invocation_ranges::InvocationRangeManager;

async fn fixture() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    ensure_schema(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn preparation_owner_query_uses_activity_index_without_sorting_or_full_scan() {
    use prompt_cache_conversations::invocation_ranges::STARTUP_PREPARATION_QUERY;
    let pool = fixture().await;
    let rows = sqlx::query(&format!("EXPLAIN QUERY PLAN {STARTUP_PREPARATION_QUERY}"))
        .bind("2026-10-01 00:00:00")
        .fetch_all(&pool)
        .await
        .unwrap();
    let details: Vec<String> = rows.iter().map(|row| row.get("detail")).collect();
    assert!(
        details
            .iter()
            .any(|detail| detail
                .contains("USING INDEX idx_prompt_cache_conversations_last_invocation")),
        "{details:?}"
    );
    assert!(
        details.iter().all(|detail| !detail.contains("TEMP B-TREE")
            && !detail.contains("SCAN prompt_cache_conversations")),
        "{details:?}"
    );
    assert!(STARTUP_PREPARATION_QUERY.ends_with("LIMIT 128"));
}

async fn existing_conversation(pool: &SqlitePool, key: &str) {
    prompt_cache_conversations::ensure_prompt_cache_conversation_row(pool, key)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE prompt_cache_conversations SET last_invocation_at=? WHERE prompt_cache_key=?",
    )
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(key)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn startup_prepares_existing_owner_without_an_incoming_request() {
    let pool = fixture().await;
    existing_conversation(&pool, "recovery-startup").await;
    let manager = Arc::new(InvocationRangeManager::default());
    let shutdown = CancellationToken::new();
    manager.start_sizing(&pool, shutdown.clone());
    // Before the fix start_sizing only estimated capacity; this committed
    // ceiling never advanced until a request initialized the owner.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let ceiling: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='recovery-startup'")
                .fetch_one(&pool).await.unwrap();
            if ceiling == 63 { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }).await.expect("existing active owner must be prepared off-request");
    shutdown.cancel();
    pool.close().await;
    let id = manager
        .allocate(&pool, Some("recovery-startup"))
        .await
        .unwrap();
    assert_eq!(
        &id[6..],
        prompt_cache_conversations::encode_prompt_cache_conversation_sequence(0).unwrap()
    );
}

#[tokio::test]
async fn hour_preparation_obeys_rollover_window_and_issues_after_database_closes() {
    let pool = fixture().await;
    let manager = Arc::new(InvocationRangeManager::default());
    let hour = Utc::now().timestamp().div_euclid(3600);
    manager
        .test_prepare_at(
            &pool,
            DateTime::from_timestamp(hour * 3600 + 3539, 0).unwrap(),
        )
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    manager
        .test_prepare_at(
            &pool,
            DateTime::from_timestamp(hour * 3600 + 3540, 0).unwrap(),
        )
        .await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM hourly_invoke_prefixes")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );
    pool.close().await;
    let first = manager.test_allocate_hour(&pool, hour).await.unwrap();
    let next = manager.test_allocate_hour(&pool, hour + 1).await.unwrap();
    assert_ne!(&first[..6], &next[..6]);
    assert!(first.ends_with(
        &prompt_cache_conversations::encode_prompt_cache_conversation_sequence(0).unwrap()
    ));
    assert!(next.ends_with(
        &prompt_cache_conversations::encode_prompt_cache_conversation_sequence(0).unwrap()
    ));
}

#[tokio::test]
async fn speculative_preparation_is_bounded_existing_only_and_yields_to_demand() {
    use crate::proxy_sqlite_write_coordinator::{
        ProxySqliteWriteClass, proxy_sqlite_write_coordinator,
    };
    let pool = fixture().await;
    let manager = Arc::new(InvocationRangeManager::default());
    assert!(!manager.test_prepare_key(&pool, "absent-owner").await);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM prompt_cache_conversations")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    existing_conversation(&pool, "priority-owner").await;
    let foreground = proxy_sqlite_write_coordinator()
        .acquire(ProxySqliteWriteClass::P1Terminal)
        .await;
    assert!(!manager.test_prepare_key(&pool, "priority-owner").await);
    drop(foreground);
    assert!(manager.test_prepare_key(&pool, "priority-owner").await);
    for number in 1..128 {
        let key = format!("bounded-warm-{number}");
        existing_conversation(&pool, &key).await;
        assert!(manager.test_prepare_key(&pool, &key).await);
    }
    existing_conversation(&pool, "warm-overflow").await;
    assert!(!manager.test_prepare_key(&pool, "warm-overflow").await);
    assert_eq!(manager.occupancy(), 128);
    let id = manager
        .allocate(&pool, Some("warm-overflow"))
        .await
        .unwrap();
    assert_eq!(
        id.len(),
        10,
        "real missing-range demand can displace speculative capacity"
    );
}

#[tokio::test]
async fn preparation_retains_owner_while_initialization_is_in_flight() {
    use prompt_cache_conversations::invocation_ranges::PREPARATION_BEFORE_READ;
    use tokio::sync::Notify;

    let pool = fixture().await;
    let key = "inflight-preparation-owner";
    existing_conversation(&pool, key).await;
    let manager = Arc::new(InvocationRangeManager::default());
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let preparation = PREPARATION_BEFORE_READ.scope(
        (entered.clone(), resume.clone()),
        manager.test_prepare_key(&pool, key),
    );
    tokio::pin!(preparation);
    tokio::select! {
        result = &mut preparation => panic!("preparation skipped the read gate: {result}"),
        _ = entered.notified() => {},
    }

    assert_eq!(
        manager.test_prepare_keys(&pool, &[key]).await,
        1,
        "an in-flight initialization must remain eligible for a later preparation turn"
    );

    resume.notify_one();
    assert!(preparation.await);
    let ceiling: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ceiling, 63);
}

#[tokio::test]
async fn deferred_preparation_retries_after_writer_pressure_without_duplicate_ranges() {
    use crate::proxy_sqlite_write_coordinator::{
        ProxySqliteWriteClass, proxy_sqlite_write_coordinator,
    };

    let pool = fixture().await;
    let key = "writer-pressure-preparation-owner";
    existing_conversation(&pool, key).await;
    let manager = Arc::new(InvocationRangeManager::default());
    let foreground = proxy_sqlite_write_coordinator()
        .acquire(ProxySqliteWriteClass::P1Terminal)
        .await;

    assert_eq!(manager.test_prepare_keys(&pool, &[key]).await, 1);
    assert_eq!(manager.test_prepare_keys(&pool, &[key, key]).await, 1);
    let before_retry: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        before_retry, -1,
        "deferred preparation cannot publish a range"
    );

    drop(foreground);
    assert_eq!(
        manager.test_retry_preparations(&pool, Utc::now()).await,
        0,
        "pressure relief must drain the retained owner"
    );
    assert_eq!(manager.test_prepare_keys(&pool, &[key, key]).await, 0);
    let after_retry: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after_retry, 63, "retries publish only one committed range");
}

#[tokio::test]
async fn deferred_preparation_retries_after_pool_pressure() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    ensure_schema(&pool).await.unwrap();
    let key = "pool-pressure-preparation-owner";
    existing_conversation(&pool, key).await;
    let manager = Arc::new(InvocationRangeManager::default());
    let held_connection = pool.acquire().await.unwrap();

    assert_eq!(manager.test_prepare_keys(&pool, &[key]).await, 1);
    drop(held_connection);
    assert_eq!(
        manager.test_retry_preparations(&pool, Utc::now()).await,
        0,
        "a later turn must retry after a pool connection becomes available"
    );
    let ceiling: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ceiling, 63);
}

#[tokio::test]
async fn deferred_preparation_retries_after_memory_capacity_pressure() {
    let pool = fixture().await;
    let key = "capacity-pressure-preparation-owner";
    existing_conversation(&pool, key).await;
    let manager = Arc::new(InvocationRangeManager::default());
    manager.test_set_preparation_target(0);

    assert_eq!(manager.test_prepare_keys(&pool, &[key]).await, 1);
    let before_retry: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before_retry, -1, "capacity deferral cannot publish a range");

    manager.test_set_preparation_target(128);
    assert_eq!(
        manager.test_retry_preparations(&pool, Utc::now()).await,
        0,
        "the retained owner must retry after memory capacity is restored"
    );
    let after_retry: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(after_retry, 63);
}

#[tokio::test]
async fn startup_retry_queue_is_bounded_and_reports_overflow() {
    use crate::proxy_sqlite_write_coordinator::{
        ProxySqliteWriteClass, proxy_sqlite_write_coordinator,
    };
    const RETRY_QUEUE_LIMIT: usize = 128;

    let pool = fixture().await;
    let manager = Arc::new(InvocationRangeManager::default());
    let owners: Vec<String> = (0..RETRY_QUEUE_LIMIT + 2)
        .map(|number| format!("retry-capacity-owner-{number}"))
        .collect();
    for key in &owners {
        existing_conversation(&pool, key).await;
    }
    let keys: Vec<&str> = owners.iter().map(String::as_str).collect();
    let foreground = proxy_sqlite_write_coordinator()
        .acquire(ProxySqliteWriteClass::P1Terminal)
        .await;

    assert_eq!(
        manager.test_prepare_keys(&pool, &keys).await,
        RETRY_QUEUE_LIMIT,
        "queue overflow must not exceed the configured owner bound"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM prompt_cache_conversations WHERE last_invoke_sequence >= 0",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0,
        "owners deferred under writer pressure cannot publish ranges"
    );
    drop(foreground);
}

#[tokio::test]
async fn startup_coordinator_retries_deferred_owners_on_its_periodic_turn() {
    use crate::proxy_sqlite_write_coordinator::{
        ProxySqliteWriteClass, proxy_sqlite_write_coordinator,
    };

    let pool = fixture().await;
    let key = "periodic-coordinator-preparation-owner";
    existing_conversation(&pool, key).await;
    let manager = Arc::new(InvocationRangeManager::default());
    let shutdown = CancellationToken::new();
    let foreground = proxy_sqlite_write_coordinator()
        .acquire(ProxySqliteWriteClass::P1Terminal)
        .await;
    manager.start_sizing(&pool, shutdown.clone());

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if manager.test_deferred_retry_count() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the startup coordinator must retain the deferred owner");
    let before_retry: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before_retry, -1);

    drop(foreground);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ceiling: i64 = sqlx::query_scalar(
                "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
            )
            .bind(key)
            .fetch_one(&pool)
            .await
            .unwrap();
            if ceiling == 63 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the existing one-second coordinator must retry deferred work");
    shutdown.cancel();
}

#[tokio::test]
async fn refill_completion_drives_the_next_eligible_batch_without_new_requests() {
    let pool = fixture().await;
    let manager = Arc::new(InvocationRangeManager::default());
    for number in 0..65 {
        let key = format!("refill-recovery-{number}");
        for _ in 0..17 {
            manager.allocate(&pool, Some(&key)).await.unwrap();
        }
    }
    // All 65 owners now have 47 IDs: eligible to join, but none has yet
    // crossed the strict <32 trigger. One trigger starts a <=64-owner batch.
    for _ in 0..16 {
        manager
            .allocate(&pool, Some("refill-recovery-0"))
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let ready: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM prompt_cache_conversations WHERE last_invoke_sequence=127",
            )
            .fetch_one(&pool)
            .await
            .unwrap();
            if ready == 65 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the 65th owner must not need another incoming request");
}

#[tokio::test]
async fn prepared_range_survives_writer_block_and_restart_never_reissues_it() {
    use crate::proxy_sqlite_write_coordinator::{
        ProxySqliteWriteClass, proxy_sqlite_write_coordinator,
    };
    let pool = fixture().await;
    existing_conversation(&pool, "prepared-restart").await;
    let manager = Arc::new(InvocationRangeManager::default());
    assert!(manager.test_prepare_key(&pool, "prepared-restart").await);
    let foreground = proxy_sqlite_write_coordinator()
        .acquire(ProxySqliteWriteClass::P1Terminal)
        .await;
    let id = manager
        .allocate(&pool, Some("prepared-restart"))
        .await
        .unwrap();
    let missing = manager.allocate(&pool, Some("new-blocked-owner")).await;
    assert!(missing.unwrap_err().to_string().contains("100ms"));
    drop(foreground);
    let restarted = Arc::new(InvocationRangeManager::default());
    let next = restarted
        .allocate(&pool, Some("prepared-restart"))
        .await
        .unwrap();
    assert_eq!(&id[..6], &next[..6]);
    assert_eq!(
        &next[6..],
        prompt_cache_conversations::encode_prompt_cache_conversation_sequence(64).unwrap()
    );
}

#[tokio::test]
async fn ended_hour_retirement_preserves_active_and_pending_invocation_obligations() {
    use prompt_cache_conversations::invocation_ranges::lifecycle::{
        PendingIdentityRegistry, prefix_occupied,
    };
    let pool = fixture().await;
    let manager = Arc::new(InvocationRangeManager::default());
    let hour = Utc::now().timestamp().div_euclid(3600);
    manager
        .test_prepare_at(&pool, DateTime::from_timestamp(hour * 3600, 0).unwrap())
        .await;
    let id = manager.test_allocate_hour(&pool, hour).await.unwrap();
    let pending = PendingIdentityRegistry::default();
    pending.register(&id, "2026-10-17 00:00:00", false);
    let next = DateTime::from_timestamp((hour + 1) * 3600, 0).unwrap();
    manager.test_prepare_at(&pool, next).await;
    assert!(
        prefix_occupied(&id[..6]),
        "active and pending owners keep their namespace"
    );
    manager.reconcile_persistence(&id);
    manager.test_prepare_at(&pool, next).await;
    assert!(
        prefix_occupied(&id[..6]),
        "journal obligation survives active lease release"
    );
    pending.acknowledge(&id, "2026-10-17 00:00:00", false);
    manager.test_prepare_at(&pool, next).await;
    assert!(
        !prefix_occupied(&id[..6]),
        "only fully idle ended hours retire from memory"
    );
    let ceiling: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM hourly_invoke_prefixes WHERE utc_hour=?",
    )
    .bind(hour)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        ceiling, 63,
        "memory eviction never returns a committed reservation ceiling"
    );
}

#[tokio::test]
async fn removed_seed_owner_does_not_starve_remaining_preparation() {
    let pool = fixture().await;
    existing_conversation(&pool, "remaining-seed").await;
    let manager = Arc::new(InvocationRangeManager::default());
    assert_eq!(
        manager
            .test_prepare_keys(&pool, &["deleted-seed", "remaining-seed"])
            .await,
        0
    );
    let ceiling: i64 = sqlx::query_scalar("SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key='remaining-seed'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(ceiling, 63);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM prompt_cache_conversations WHERE prompt_cache_key='deleted-seed'"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        0,
        "preparation cannot recreate a deleted conversation"
    );
}

#[tokio::test]
async fn demand_retries_when_an_inflight_preparation_owner_was_deleted() {
    use prompt_cache_conversations::invocation_ranges::PREPARATION_BEFORE_READ;
    use tokio::sync::Notify;
    let pool = fixture().await;
    let key = "deleted-during-preparation";
    existing_conversation(&pool, key).await;
    let manager = Arc::new(InvocationRangeManager::default());
    let entered = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let preparation_manager = manager.clone();
    let preparation_pool = pool.clone();
    let preparation_key = key.to_owned();
    let preparation = tokio::spawn(PREPARATION_BEFORE_READ.scope(
        (entered.clone(), resume),
        async move {
            preparation_manager
                .test_prepare_key(&preparation_pool, &preparation_key)
                .await
        },
    ));
    entered.notified().await;
    sqlx::query("DELETE FROM prompt_cache_conversations WHERE prompt_cache_key=?")
        .bind(key)
        .execute(&pool)
        .await
        .unwrap();
    let invoke_id = tokio::time::timeout(
        Duration::from_millis(150),
        manager.allocate(&pool, Some(key)),
    )
    .await
    .expect("live demand promotes before its 100ms budget")
    .expect("real demand retries normal committed initialization");
    assert!(
        !preparation.await.unwrap(),
        "a deleted seed owner is not prepared"
    );
    assert_eq!(invoke_id.len(), 10);
    let ceiling: i64 = sqlx::query_scalar(
        "SELECT last_invoke_sequence FROM prompt_cache_conversations WHERE prompt_cache_key=?",
    )
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ceiling, 63, "demand publishes only a committed reservation");
}
