use super::runtime_overlay_and_group_rule_behaviors::test_proxy_capture_record;
use super::*;

const REFRESH_MARKER: &str = "prompt_cache_working_set_relevant_updates_v1";

async fn working_set_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(":memory:")
        .await
        .expect("open isolated working-set database");
    ensure_schema(&pool).await.expect("initialize schema");
    pool
}

async fn seed_live_capture(pool: &SqlitePool, invoke_id: &str) {
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let record = test_proxy_capture_record(invoke_id, &occurred_at);
    let persisted =
        crate::proxy::persist_proxy_capture_record_core(pool, Instant::now(), record, false)
            .await
            .expect("commit production terminal write")
            .expect("terminal invocation exists");
    assert!(persisted.t_persist_ms.is_some());
}

async fn projection_snapshot(pool: &SqlitePool) -> String {
    // Compare every business column, including nullable date and source-scope fields.
    // The refresh timestamp is bookkeeping and can change during the reference rebuild.
    let columns: Vec<String> = sqlx::query("PRAGMA table_info(prompt_cache_working_set_live)")
        .fetch_all(pool)
        .await
        .expect("read projection columns")
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .filter(|name| name != "updated_at")
        .collect();
    let fields = columns
        .iter()
        .map(|name| format!("'{name}', {name}"))
        .collect::<Vec<_>>()
        .join(", ");
    sqlx::query_scalar(&format!(
        "SELECT json_group_array(json_object({fields})) FROM \
         (SELECT * FROM prompt_cache_working_set_live ORDER BY prompt_cache_key)"
    ))
    .fetch_one(pool)
    .await
    .expect("snapshot working-set projection")
}

#[tokio::test]
async fn prompt_working_set_terminal_timing_write_does_not_recompute_projection() {
    let pool = working_set_pool().await;
    // The first source INSERT legitimately inserts its projection. The production timing
    // follow-up must not UPDATE that projection or scan the same key again inside P1.
    sqlx::query(
        "CREATE TRIGGER reject_redundant_projection_update \
         BEFORE UPDATE ON prompt_cache_working_set_live BEGIN \
         SELECT RAISE(ABORT, 'timing-only write recomputed the working set'); END",
    )
    .execute(&pool)
    .await
    .expect("install redundant-work sentinel");
    seed_live_capture(&pool, "working-set-terminal-timing").await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT request_count FROM prompt_cache_working_set_live")
            .fetch_one(&pool)
            .await
            .expect("read committed projection"),
        1
    );
}

#[tokio::test]
async fn prompt_working_set_business_mutations_match_full_rebuild() {
    let pool = working_set_pool().await;
    seed_live_capture(&pool, "working-set-mutations").await;
    for mutation in [
        "UPDATE codex_invocations SET total_tokens = total_tokens",
        "UPDATE codex_invocations SET total_tokens = 81",
        "UPDATE codex_invocations SET cost = 0.75",
        "UPDATE codex_invocations SET status = 'running'",
        "UPDATE codex_invocations SET error_message = 'request failed'",
        "UPDATE codex_invocations SET failure_kind = 'proxy_interrupted'",
        "UPDATE codex_invocations SET failure_class = 'upstream'",
        "UPDATE codex_invocations SET status = 'success', error_message = NULL, failure_kind = NULL, failure_class = NULL",
        "UPDATE codex_invocations SET source = 'xy'",
        "UPDATE codex_invocations SET occurred_at = datetime(occurred_at, '-10 seconds')",
        "UPDATE codex_invocations SET payload = '{\"promptCacheKey\":\" moved-key \"}'",
        "UPDATE codex_invocations SET invoke_id = 'working-set-renamed', id = id + 100",
        "UPDATE codex_invocations SET payload = '{\"promptCacheKey\":\"\"}'",
        "UPDATE codex_invocations SET payload = '{\"promptCacheKey\":\"restored-key\"}'",
        "DELETE FROM codex_invocations",
    ] {
        sqlx::query(mutation)
            .execute(&pool)
            .await
            .unwrap_or_else(|err| panic!("apply {mutation}: {err}"));
        let incremental = projection_snapshot(&pool).await;
        crate::schema::rebuild_prompt_cache_working_set_live_table(&pool)
            .await
            .expect("rebuild reference projection from source rows");
        assert_eq!(incremental, projection_snapshot(&pool).await, "{mutation}");
    }
}

#[tokio::test]
async fn prompt_working_set_large_history_refresh_reads_only_recent_and_live_source_rows() {
    let pool = working_set_pool().await;
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<10000)
         INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response)
         SELECT 'history-'||i,datetime('now','+8 hours','-1 day'),'proxy','success',7,0.07,
             '{\"promptCacheKey\":\"large-working-key\"}','{}' FROM n",
    )
    .execute(&pool)
    .await
    .expect("large terminal history");
    let instructions = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let mut connection = pool
        .acquire()
        .await
        .expect("instrumented source connection");
    {
        let counter = instructions.clone();
        let mut handle = connection.lock_handle().await.expect("SQLite handle");
        handle.set_progress_handler(1_000, move || {
            counter.fetch_add(1_000, std::sync::atomic::Ordering::Relaxed) < 100_000
        });
    }
    let result = sqlx::query(
        "INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response)
         VALUES('new-live',datetime('now','+8 hours'),'proxy','success',7,0.07,
             '{\"promptCacheKey\":\"large-working-key\"}','{}')",
    )
    .execute(&mut *connection)
    .await;
    connection
        .lock_handle()
        .await
        .expect("remove diagnostic handler")
        .remove_progress_handler();
    result.expect("online refresh must not evaluate every historical payload");
    drop(connection);
    for mutation in [
        "INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,payload,raw_response) VALUES('old-running',datetime('now','+8 hours','-1 day'),'proxy',' running ','{\"promptCacheKey\":\"large-working-key\"}','{}')",
        "INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,payload,raw_response) VALUES('old-pending',datetime('now','+8 hours','-1 day'),'xy','pending','{\"promptCacheKey\":\"large-working-key\"}','{}')",
        "UPDATE codex_invocations SET id=id+100000,payload='{\"promptCacheKey\":\" moved-key \"}' WHERE invoke_id='old-running'",
        "UPDATE codex_invocations SET failure_class='service_failure' WHERE invoke_id='old-running'",
        "UPDATE codex_invocations SET status='failed',error_message='failed',occurred_at=datetime('now','+8 hours','-1 day') WHERE invoke_id='new-live'",
        "DELETE FROM codex_invocations WHERE invoke_id='old-pending'",
    ] {
        sqlx::query(mutation)
            .execute(&pool)
            .await
            .expect("source mutation");
        let incremental = projection_snapshot(&pool).await;
        crate::schema::rebuild_prompt_cache_working_set_live_table(&pool)
            .await
            .expect("exact source reference");
        assert_eq!(incremental, projection_snapshot(&pool).await, "{mutation}");
    }
    pool.close().await;
}

#[tokio::test]
async fn prompt_working_set_installed_marker_does_not_hide_obsolete_trigger_definition() {
    let pool = working_set_pool().await;
    seed_live_capture(&pool, "working-set-definition-upgrade").await;
    let before = projection_snapshot(&pool).await;
    sqlx::query("UPDATE schema_refresh_migrations SET completed_at='2020-01-01 00:00:00' WHERE migration_name=?1")
        .bind(REFRESH_MARKER).execute(&pool).await.expect("immutable deployed completion fact");
    sqlx::query("DROP TRIGGER trg_codex_invocations_prompt_cache_working_set_update")
        .execute(&pool)
        .await
        .expect("simulate incomplete prior definition");
    sqlx::query(
        "CREATE TRIGGER trg_codex_invocations_prompt_cache_working_set_update
         AFTER UPDATE ON codex_invocations BEGIN SELECT 1; END",
    )
    .execute(&pool)
    .await
    .expect("obsolete definition with retained marker");
    sqlx::query(
        "CREATE TRIGGER reject_definition_row_rebuild BEFORE DELETE ON prompt_cache_working_set_live
         BEGIN SELECT RAISE(ABORT,'definition upgrade must not rebuild rows'); END",
    ).execute(&pool).await.expect("preserve existing facts");
    ensure_schema(&pool)
        .await
        .expect("repair definition despite installed marker");
    let sql: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE name='trg_codex_invocations_prompt_cache_working_set_update'",
    ).fetch_one(&pool).await.expect("installed definition");
    assert!(sql.contains("SELECT invocation_id FROM invocation_in_progress_live"));
    assert!(sql.contains("UNION SELECT NEW.id"));
    assert_eq!(projection_snapshot(&pool).await, before);
    ensure_schema(&pool).await.expect("repeat definition check");
    assert_eq!(projection_snapshot(&pool).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT completed_at FROM schema_refresh_migrations WHERE migration_name=?1"
        )
        .bind(REFRESH_MARKER)
        .fetch_one(&pool)
        .await
        .expect("preserved completion fact"),
        "2020-01-01 00:00:00"
    );
}

async fn install_legacy_update_trigger(pool: &SqlitePool) -> String {
    let mut legacy: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE name = \
         'trg_codex_invocations_prompt_cache_working_set_update'",
    )
    .fetch_one(pool)
    .await
    .expect("read installed trigger");
    let update = legacy.find("AFTER UPDATE").expect("update trigger");
    let table = legacy[update..]
        .find("ON codex_invocations")
        .expect("source table")
        + update;
    legacy.replace_range(update..table, "AFTER UPDATE ");
    sqlx::query("DROP TRIGGER trg_codex_invocations_prompt_cache_working_set_update")
        .execute(pool)
        .await
        .expect("remove current trigger");
    sqlx::query(&legacy)
        .execute(pool)
        .await
        .expect("install broad legacy trigger");
    sqlx::query("DELETE FROM schema_refresh_migrations WHERE migration_name = ?1")
        .bind(REFRESH_MARKER)
        .execute(pool)
        .await
        .expect("restore pre-upgrade marker state");
    legacy
}

#[tokio::test]
async fn prompt_working_set_trigger_upgrade_is_atomic_and_reentrant_without_row_rebuild() {
    let pool = working_set_pool().await;
    seed_live_capture(&pool, "working-set-upgrade").await;
    let before = projection_snapshot(&pool).await;
    let legacy = install_legacy_update_trigger(&pool).await;
    sqlx::query(
        "CREATE TRIGGER reject_projection_rebuild BEFORE DELETE ON prompt_cache_working_set_live \
         BEGIN SELECT RAISE(ABORT, 'trigger upgrade must preserve live rows'); END",
    )
    .execute(&pool)
    .await
    .expect("guard business projection rows");
    sqlx::query(&format!(
        "CREATE TRIGGER interrupt_refresh_marker BEFORE INSERT ON schema_refresh_migrations \
         WHEN NEW.migration_name = '{REFRESH_MARKER}' \
         BEGIN SELECT RAISE(ABORT, 'interrupted trigger refresh'); END"
    ))
    .execute(&pool)
    .await
    .expect("inject failure immediately before completion marker");
    assert!(ensure_schema(&pool).await.is_err());
    let installed: String = sqlx::query_scalar(
        "SELECT sql FROM sqlite_schema WHERE name = \
         'trg_codex_invocations_prompt_cache_working_set_update'",
    )
    .fetch_one(&pool)
    .await
    .expect("read trigger after interrupted upgrade");
    assert_eq!(installed, legacy, "DDL must roll back with marker failure");
    assert_eq!(projection_snapshot(&pool).await, before);
    sqlx::query("DROP TRIGGER interrupt_refresh_marker")
        .execute(&pool)
        .await
        .expect("remove injected interruption");
    ensure_schema(&pool)
        .await
        .expect("retry interrupted upgrade");
    let completed_at: String = sqlx::query_scalar(
        "SELECT completed_at FROM schema_refresh_migrations WHERE migration_name = ?1",
    )
    .bind(REFRESH_MARKER)
    .fetch_one(&pool)
    .await
    .expect("upgrade completion marker");
    ensure_schema(&pool)
        .await
        .expect("repeat successful startup");
    assert_eq!(projection_snapshot(&pool).await, before);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT completed_at FROM schema_refresh_migrations WHERE migration_name = ?1"
        )
        .bind(REFRESH_MARKER)
        .fetch_one(&pool)
        .await
        .expect("read retained completion marker"),
        completed_at
    );
    sqlx::query(
        "CREATE TRIGGER reject_upgraded_timing_refresh BEFORE UPDATE ON prompt_cache_working_set_live \
         BEGIN SELECT RAISE(ABORT, 'upgraded timing write recomputed projection'); END",
    )
    .execute(&pool)
    .await
    .expect("guard upgraded projection against timing-only work");
    sqlx::query("UPDATE codex_invocations SET t_persist_ms = 42")
        .execute(&pool)
        .await
        .expect("timing-only write remains compatible after upgrade");
}
