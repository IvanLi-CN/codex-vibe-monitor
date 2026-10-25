use super::*;

#[tokio::test]
async fn ensure_schema_backfills_raw_codecs_and_manifest_tables() {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:ensure-schema-raw-codecs-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect schema sqlite");

    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            raw_response TEXT NOT NULL,
            request_raw_path TEXT,
            response_raw_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy codex_invocations");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            raw_response,
            request_raw_path,
            response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("legacy-codec-row")
    .bind("2026-03-01 08:00:00")
    .bind("{}")
    .bind("proxy_raw_payloads/request.bin")
    .bind("proxy_raw_payloads/response.bin.gz")
    .execute(&pool)
    .await
    .expect("insert legacy codec row");
    sqlx::query(
        r#"
        CREATE TABLE archive_batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            dataset TEXT NOT NULL,
            month_key TEXT NOT NULL,
            file_path TEXT NOT NULL,
            sha256 TEXT NOT NULL,
            row_count INTEGER NOT NULL,
            status TEXT NOT NULL,
            coverage_start_at TEXT,
            coverage_end_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(dataset, month_key, file_path)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy archive batch manifest");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            created_at
        )
        VALUES ('codex_invocations', '2026-03', '/tmp/legacy-coverage.sqlite.gz', 'coverage-sha', 1, 'completed', '2026-03-01 08:00:00')
        "#,
    )
    .execute(&pool)
    .await
    .expect("insert legacy archive manifest without coverage bounds");
    sqlx::query(
        r#"
        INSERT INTO archive_batches (
            dataset, month_key, file_path, sha256, row_count, status,
            coverage_start_at, coverage_end_at, created_at
        )
        VALUES (
            'codex_invocations', '2026-03', '/tmp/legacy-coverage-backfill.sqlite.gz',
            'backfill-sha', 1, 'completed', '2026-03-01 08:00:00',
            '2026-03-01 09:00:00', '2026-03-01 08:00:00'
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("insert legacy archive manifest with text coverage bounds");

    ensure_schema(&pool).await.expect("ensure schema migration");

    let row = sqlx::query(
        "SELECT request_raw_codec, response_raw_codec FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("legacy-codec-row")
    .fetch_one(&pool)
    .await
    .expect("load migrated codec row");
    assert_eq!(
        row.get::<String, _>("request_raw_codec"),
        RAW_CODEC_IDENTITY
    );
    assert_eq!(row.get::<String, _>("response_raw_codec"), RAW_CODEC_GZIP);

    let completed_migrations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocation_raw_codec_migrations WHERE migration_name = 'backfill_raw_codecs_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load raw codec migration marker");
    assert_eq!(completed_migrations, 1);

    let completed_plan: Vec<String> = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM codex_invocation_raw_codec_migrations WHERE migration_name = 'backfill_raw_codecs_v1')",
    )
    .fetch_all(&pool)
    .await
    .expect("explain raw codec completion marker lookup")
    .into_iter()
    .map(|row| row.get("detail"))
    .collect();
    assert!(
        completed_plan
            .iter()
            .any(|detail| detail.contains("SEARCH codex_invocation_raw_codec_migrations")),
        "completed startup must seek the raw codec marker: {completed_plan:?}"
    );
    assert!(
        completed_plan
            .iter()
            .all(|detail| !detail.contains("codex_invocations")),
        "completed codec marker lookup must not scan invocations: {completed_plan:?}"
    );
    sqlx::query("UPDATE codex_invocations SET response_raw_codec = ?1 WHERE invoke_id = ?2")
        .bind(RAW_CODEC_IDENTITY)
        .bind("legacy-codec-row")
        .execute(&pool)
        .await
        .expect("simulate a post-migration row");
    sqlx::query(
        r#"
        CREATE TRIGGER fail_completed_raw_codec_updates
        BEFORE UPDATE OF request_raw_codec, response_raw_codec ON codex_invocations
        BEGIN
            SELECT RAISE(ABORT, 'completed codec migration attempted an invocation update');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install completed migration update guard");
    ensure_schema(&pool)
        .await
        .expect("completed codec migration should be idempotent");
    let response_codec: String =
        sqlx::query_scalar("SELECT response_raw_codec FROM codex_invocations WHERE invoke_id = ?1")
            .bind("legacy-codec-row")
            .fetch_one(&pool)
            .await
            .expect("load idempotent codec row");
    assert_eq!(response_codec, RAW_CODEC_IDENTITY);

    let archive_batch_columns = load_sqlite_table_columns(&pool, "archive_batches")
        .await
        .expect("load archive batch columns");
    assert!(archive_batch_columns.contains("upstream_activity_manifest_refreshed_at"));
    assert!(archive_batch_columns.contains("coverage_start_epoch"));
    assert!(archive_batch_columns.contains("coverage_end_epoch"));
    sqlx::query(
        "UPDATE archive_batches SET coverage_start_at = '2026-03-01 08:00:00', coverage_end_at = '2026-03-01T01:00:00Z' WHERE file_path = '/tmp/legacy-coverage.sqlite.gz'",
    )
    .execute(&pool)
    .await
    .expect("update archive coverage bounds through epoch trigger");
    let normalized_coverage = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT coverage_start_epoch, coverage_end_epoch FROM archive_batches WHERE file_path = '/tmp/legacy-coverage.sqlite.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("load normalized archive coverage");
    assert_eq!(normalized_coverage.0, Some(1_772_323_200));
    assert_eq!(normalized_coverage.1, Some(1_772_326_800));
    let backfilled_coverage = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT coverage_start_epoch, coverage_end_epoch FROM archive_batches WHERE file_path = '/tmp/legacy-coverage-backfill.sqlite.gz'",
    )
    .fetch_one(&pool)
    .await
    .expect("load backfilled archive coverage");
    assert_eq!(
        backfilled_coverage,
        (Some(1_772_323_200), Some(1_772_326_800))
    );
    let coverage_index: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_archive_batches_invocation_coverage_epoch'",
    )
    .fetch_optional(&pool)
    .await
    .expect("load archive coverage index");
    assert_eq!(
        coverage_index.as_deref(),
        Some("idx_archive_batches_invocation_coverage_epoch")
    );
    let manifest_columns = load_sqlite_table_columns(&pool, "archive_batch_upstream_activity")
        .await
        .expect("load manifest columns");
    assert!(manifest_columns.contains("archive_batch_id"));
    assert!(manifest_columns.contains("account_id"));
    assert!(manifest_columns.contains("last_activity_at"));
}

#[tokio::test]
async fn ensure_schema_commits_raw_codec_backfill_and_marker_atomically() {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:raw-codec-atomic-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect schema sqlite");

    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            raw_response TEXT NOT NULL,
            request_raw_path TEXT,
            request_raw_codec TEXT NOT NULL DEFAULT 'identity',
            response_raw_path TEXT,
            response_raw_codec TEXT NOT NULL DEFAULT 'identity',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy codex_invocations");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            raw_response,
            request_raw_path,
            response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("atomic-codec-row")
    .bind("2026-03-01 08:00:00")
    .bind("{}")
    .bind("proxy_raw_payloads/request.bin.gz")
    .bind("proxy_raw_payloads/response.bin.gz")
    .execute(&pool)
    .await
    .expect("insert legacy codec row");
    sqlx::query(
        r#"
        CREATE TRIGGER fail_response_raw_codec_backfill
        BEFORE UPDATE OF response_raw_codec ON codex_invocations
        WHEN NEW.response_raw_codec = 'gzip'
        BEGIN
            SELECT RAISE(ABORT, 'forced raw codec backfill failure');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install raw codec failure trigger");

    let error = ensure_schema(&pool)
        .await
        .expect_err("response codec failure should abort the migration");
    assert!(
        format!("{error:#}").contains("failed to backfill codex_invocations response_raw_codec"),
        "unexpected migration error: {error:#}"
    );

    let row = sqlx::query(
        "SELECT request_raw_codec, response_raw_codec FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("atomic-codec-row")
    .fetch_one(&pool)
    .await
    .expect("load rolled-back codec row");
    assert_eq!(
        row.get::<String, _>("request_raw_codec"),
        RAW_CODEC_IDENTITY
    );
    assert_eq!(
        row.get::<String, _>("response_raw_codec"),
        RAW_CODEC_IDENTITY
    );
    let completed_migrations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocation_raw_codec_migrations WHERE migration_name = 'backfill_raw_codecs_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load raw codec migration marker after rollback");
    assert_eq!(completed_migrations, 0);
}

#[tokio::test]
async fn ensure_schema_honors_legacy_raw_blob_seed_marker_for_raw_codecs() {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:raw-codec-legacy-marker-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect schema sqlite");

    ensure_schema(&pool)
        .await
        .expect("initialize schema and raw blob seed marker");
    sqlx::query(
        "DELETE FROM codex_invocation_raw_codec_migrations WHERE migration_name = 'backfill_raw_codecs_v1'",
    )
    .execute(&pool)
    .await
    .expect("remove new codec marker to simulate an upgraded legacy database");
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            raw_response,
            response_raw_path,
            response_raw_codec
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind("legacy-seed-proof-row")
    .bind("2026-03-01 08:00:00")
    .bind("{}")
    .bind("proxy_raw_payloads/response.bin.gz")
    .bind(RAW_CODEC_IDENTITY)
    .execute(&pool)
    .await
    .expect("insert row that would expose a repeated codec scan");
    sqlx::query(
        r#"
        CREATE TRIGGER fail_repeated_response_raw_codec_backfill
        BEFORE UPDATE OF response_raw_codec ON codex_invocations
        WHEN NEW.response_raw_codec = 'gzip'
        BEGIN
            SELECT RAISE(ABORT, 'repeated raw codec backfill');
        END
        "#,
    )
    .execute(&pool)
    .await
    .expect("install repeated-backfill failure trigger");

    ensure_schema(&pool)
        .await
        .expect("legacy raw blob seed marker should skip codec scans");

    let response_codec: String =
        sqlx::query_scalar("SELECT response_raw_codec FROM codex_invocations WHERE invoke_id = ?1")
            .bind("legacy-seed-proof-row")
            .fetch_one(&pool)
            .await
            .expect("load untouched codec row");
    assert_eq!(response_codec, RAW_CODEC_IDENTITY);
    let completed_migrations: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocation_raw_codec_migrations WHERE migration_name = 'backfill_raw_codecs_v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("load restored raw codec migration marker");
    assert_eq!(completed_migrations, 1);
}

#[tokio::test]
async fn fetch_stats_exposes_maintenance_observability_fields() {
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
    .bind(812_i64)
    .bind("api_key_codex")
    .bind("codex")
    .bind("Stats maintenance account")
    .bind("active")
    .bind(1_i64)
    .bind(&created_at)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    .expect("insert stats maintenance account");
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
    .bind("stats-maintenance-row")
    .bind(shanghai_local_days_ago(3, 8, 0, 0))
    .bind(SOURCE_PROXY)
    .bind(12_i64)
    .bind(0.1_f64)
    .bind("success")
    .bind("{}")
    .bind("proxy_raw_payloads/stats-maintenance.bin")
    .bind(RAW_CODEC_IDENTITY)
    .bind(2048_i64)
    .execute(&state.pool)
    .await
    .expect("insert stats maintenance invocation");
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
    .bind("stats-maintenance-hot-row")
    .bind(shanghai_local_now_minus_secs(20 * 60))
    .bind(SOURCE_PROXY)
    .bind(6_i64)
    .bind(0.05_f64)
    .bind("success")
    .bind("{}")
    .bind("proxy_raw_payloads/stats-maintenance-hot.bin")
    .bind(RAW_CODEC_IDENTITY)
    .bind(4096_i64)
    .execute(&state.pool)
    .await
    .expect("insert hot raw invocation that should not count as backlog");

    hydrate_stats_snapshot_for_test(&state).await;
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

#[tokio::test]
async fn fetch_stats_reuses_cached_maintenance_snapshot_within_ttl() {
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

    hydrate_stats_snapshot_for_test(&state).await;
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
