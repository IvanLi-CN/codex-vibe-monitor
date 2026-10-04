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
