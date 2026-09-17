#[tokio::test]
pub(crate) async fn fetch_stats_exposes_maintenance_observability_fields() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    seed_stats_maintenance_observability(&state.pool).await;

    let Json(stats) = fetch_stats(State(state))
        .await
        .expect("fetch stats with maintenance payload");
    let maintenance = stats.maintenance.expect("stats maintenance payload");
    let raw_backlog = maintenance
        .raw_compression_backlog
        .expect("raw backlog maintenance payload");
    assert_eq!(raw_backlog.uncompressed_count, 1);
    assert_eq!(raw_backlog.uncompressed_bytes, 2048);
    assert_eq!(raw_backlog.alert_level, RawCompressionAlertLevel::Critical);
    let startup_backfill = maintenance
        .startup_backfill
        .expect("startup backfill maintenance payload");
    assert_eq!(
        startup_backfill.upstream_activity_archive_pending_accounts,
        1
    );
    assert_eq!(startup_backfill.zero_update_streak, 0);
    assert!(startup_backfill.next_run_after.is_none());
    let historical_rollup_backfill = maintenance
        .historical_rollup_backfill
        .expect("historical rollup backfill maintenance payload");
    assert_eq!(historical_rollup_backfill.pending_buckets, 0);
    assert_eq!(historical_rollup_backfill.legacy_archive_pending, 0);
    assert!(historical_rollup_backfill.last_materialized_hour.is_none());
    assert_eq!(
        historical_rollup_backfill.alert_level,
        HistoricalRollupBackfillAlertLevel::None
    );
}

async fn seed_stats_maintenance_observability(pool: &SqlitePool) {
    insert_stats_maintenance_account(pool, 812, "Stats maintenance account").await;
    insert_stats_maintenance_invocation(
        pool,
        "stats-maintenance-row",
        shanghai_local_days_ago(3, 8, 0, 0),
        12,
        0.1,
        "proxy_raw_payloads/stats-maintenance.bin",
        2048,
    )
    .await;
    insert_stats_maintenance_invocation(
        pool,
        "stats-maintenance-hot-row",
        shanghai_local_now_minus_secs(20 * 60),
        6,
        0.05,
        "proxy_raw_payloads/stats-maintenance-hot.bin",
        4096,
    )
    .await;
}

async fn insert_stats_maintenance_account(pool: &SqlitePool, id: i64, display_name: &str) {
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(id)
    .bind("api_key_codex")
    .bind("codex")
    .bind(display_name)
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(pool)
    .await
    .expect("insert stats maintenance account");
}

async fn insert_stats_maintenance_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: String,
    total_tokens: i64,
    cost: f64,
    request_raw_path: &str,
    request_raw_size: i64,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(total_tokens)
    .bind(cost)
    .bind("success")
    .bind("{}")
    .bind(request_raw_path)
    .bind(RAW_CODEC_IDENTITY)
    .bind(request_raw_size)
    .execute(pool)
    .await
    .expect("insert stats maintenance invocation");
}

#[tokio::test]
pub(crate) async fn fetch_stats_reuses_cached_maintenance_snapshot_within_ttl() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let created_at = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_accounts (
            id, kind, provider, display_name, status, enabled, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(813_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Cached maintenance account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert cached maintenance account");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("cached-maintenance-row")
    .bind(shanghai_local_days_ago(3, 9, 30, 0))
    .bind(SOURCE_PROXY)
    .bind(10_i64)
    .bind(0.2_f64)
    .bind("success")
    .bind("{}")
    .bind("proxy_raw_payloads/cached-maintenance.bin")
    .bind(RAW_CODEC_IDENTITY)
    .bind(4096_i64)
    .execute(&state.pool)
    .await
    .expect("insert cached maintenance invocation");

    let Json(first_stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch first stats maintenance snapshot");

    sqlx::query("UPDATE codex_invocations SET request_raw_codec = ?1 WHERE invoke_id = ?2")
        .bind(RAW_CODEC_GZIP)
        .bind("cached-maintenance-row")
        .execute(&state.pool)
        .await
        .expect("mark cached maintenance invocation compressed");
    sqlx::query(
        r#"
        INSERT INTO startup_backfill_progress (
            task_name,
            cursor_id,
            next_run_after,
            zero_update_streak,
            last_started_at,
            last_finished_at,
            last_scanned,
            last_updated,
            last_status
        )
        VALUES (?1, 0, ?2, 4, NULL, NULL, 0, 0, ?3)
        ON CONFLICT(task_name) DO UPDATE SET
            next_run_after = excluded.next_run_after,
            zero_update_streak = excluded.zero_update_streak,
            last_status = excluded.last_status
        "#,
    )
    .bind(STARTUP_BACKFILL_TASK_UPSTREAM_ACTIVITY_ARCHIVES)
    .bind(format_utc_iso(Utc::now() + ChronoDuration::hours(1)))
    .bind(STARTUP_BACKFILL_STATUS_OK)
    .execute(&state.pool)
    .await
    .expect("update cached startup progress");

    let Json(second_stats) = fetch_stats(State(state))
        .await
        .expect("fetch cached stats maintenance snapshot");
    assert_eq!(second_stats.maintenance, first_stats.maintenance);
}

#[derive(Debug)]
pub(crate) struct FakeSqliteCodeDatabaseError {
    pub(crate) message: &'static str,
    pub(crate) code: &'static str,
}

impl std::fmt::Display for FakeSqliteCodeDatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FakeSqliteCodeDatabaseError {}

impl DatabaseError for FakeSqliteCodeDatabaseError {
    fn message(&self) -> &str {
        self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.code))
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

pub(crate) fn write_backfill_response_payload(path: &Path) {
    write_backfill_response_payload_with_service_tier(path, None);
}

use super::*;
