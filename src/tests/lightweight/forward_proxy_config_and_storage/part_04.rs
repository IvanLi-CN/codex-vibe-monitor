#[tokio::test]
pub(crate) async fn spawn_raw_payload_file_write_spools_when_async_writer_pool_is_saturated() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let available_permits = state.proxy_raw_async_semaphore.available_permits();
    let permit = state
        .proxy_raw_async_semaphore
        .clone()
        .acquire_many_owned(available_permits as u32)
        .await
        .expect("saturate async raw writer permits");

    let pending = spawn_raw_payload_file_write(
        state.as_ref(),
        "proxy-test",
        "request",
        Bytes::from_static(br#"{"ok":true}"#),
        true,
    );

    let spool_dir = state.config.resolved_proxy_raw_dir().join(".spool");
    assert!(
        fs::read_dir(&spool_dir)
            .expect("raw spool directory should exist")
            .next()
            .is_some(),
        "saturated capture should be persisted in the overflow spool"
    );

    drop(permit);
    let meta = pending.finish().await;

    assert!(
        meta.path.is_some(),
        "spooled raw payload should be stored after the writer becomes available"
    );
    assert_eq!(meta.size_bytes, br#"{"ok":true}"#.len() as i64);
    assert!(
        !meta.truncated,
        "spooled raw payload should retain its full contents"
    );
    assert!(
        meta.truncated_reason.is_none(),
        "successful spool replay should not report a truncation reason"
    );
}

#[test]
pub(crate) fn search_raw_script_reports_corrupt_gzip_as_error() {
    let temp_dir = make_temp_test_dir("search-raw-script-corrupt-gzip");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let gzip_path = root.join("broken.bin.gz");
    fs::write(&gzip_path, b"not-gzip").expect("write corrupt gzip file");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("needle")
    .output()
    .expect("run search-raw with corrupt gzip");

    assert_eq!(
        output.status.code(),
        Some(2),
        "corrupt gzip should be treated as hard error"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("failed to decompress"),
        "corrupt gzip should explain the decompression failure, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[test]
pub(crate) fn search_raw_script_reports_plain_file_read_errors() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_temp_test_dir("search-raw-script-plain-permission-denied");
    let root = temp_dir.join("proxy_raw_payloads");
    fs::create_dir_all(&root).expect("create raw root");
    let plain_path = root.join("plain.bin");
    fs::write(&plain_path, b"permission-token\n").expect("write plain raw");

    let mut permissions = fs::metadata(&plain_path)
        .expect("read plain raw metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&plain_path, permissions).expect("chmod plain raw");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("permission-token")
    .output()
    .expect("run search-raw with unreadable plain file");

    let mut repaired_permissions = fs::metadata(&plain_path)
        .expect("read plain raw metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o644);
    fs::set_permissions(&plain_path, repaired_permissions).expect("restore plain raw permissions");

    assert_eq!(
        output.status.code(),
        Some(2),
        "plain grep errors should be treated as hard errors"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("grep failed"),
        "plain grep failure should be explained, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
#[test]
pub(crate) fn search_raw_script_reports_find_enumeration_errors() {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_temp_test_dir("search-raw-script-find-permission-denied");
    let root = temp_dir.join("proxy_raw_payloads");
    let readable_dir = root.join("readable");
    let blocked_dir = root.join("blocked");
    fs::create_dir_all(&readable_dir).expect("create readable raw dir");
    fs::create_dir_all(&blocked_dir).expect("create blocked raw dir");
    fs::write(readable_dir.join("plain.bin"), b"permission-token\n").expect("write readable raw");

    let mut permissions = fs::metadata(&blocked_dir)
        .expect("read blocked dir metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&blocked_dir, permissions).expect("chmod blocked dir");

    let output = std::process::Command::new(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/search-raw"),
    )
    .arg("--root")
    .arg(&root)
    .arg("permission-token")
    .output()
    .expect("run search-raw with unreadable directory");

    let mut repaired_permissions = fs::metadata(&blocked_dir)
        .expect("read blocked dir metadata after run")
        .permissions();
    repaired_permissions.set_mode(0o755);
    fs::set_permissions(&blocked_dir, repaired_permissions)
        .expect("restore blocked dir permissions");

    assert_eq!(
        output.status.code(),
        Some(2),
        "find enumeration errors should be treated as hard errors"
    );
    let stderr = String::from_utf8(output.stderr).expect("search-raw stderr");
    assert!(
        stderr.contains("failed to enumerate raw files"),
        "find failures should be explained, got: {stderr}"
    );

    cleanup_temp_test_dir(&temp_dir);
}

pub(crate) fn test_sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

pub(crate) async fn retention_test_pool_and_config(
    prefix: &str,
) -> (SqlitePool, AppConfig, PathBuf) {
    let schema_template = std::env::var_os(ARCHIVE_SCHEMA_TEMPLATE_PATH_ENV).map(PathBuf::from);
    retention_test_pool_and_config_from_template(prefix, schema_template.as_deref()).await
}

pub(crate) async fn retention_fresh_schema_test_pool_and_config(
    prefix: &str,
) -> (SqlitePool, AppConfig, PathBuf) {
    retention_test_pool_and_config_with_connections(prefix, None, 1).await
}

pub(crate) async fn retention_test_pool_and_config_from_template(
    prefix: &str,
    schema_template: Option<&Path>,
) -> (SqlitePool, AppConfig, PathBuf) {
    retention_test_pool_and_config_with_connections(prefix, schema_template, 2).await
}

pub(crate) async fn retention_test_pool_and_config_with_connections(
    prefix: &str,
    schema_template: Option<&Path>,
    max_connections: u32,
) -> (SqlitePool, AppConfig, PathBuf) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    let used_schema_template = schema_template.is_some();
    if let Some(schema_template) = schema_template {
        assert!(
            schema_template.is_file(),
            "archive schema template must exist: {}",
            schema_template.display()
        );
        fs::copy(schema_template, &db_path).expect("copy archive current-schema template");
    } else {
        fs::File::create(&db_path).expect("create retention sqlite file");
    }
    let db_url = test_sqlite_url_for_path(&db_path);
    let pool = SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect(&db_url)
        .await
        .expect("connect retention sqlite");
    if !used_schema_template {
        ensure_schema(&pool).await.expect("ensure retention schema");
    }

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
    config.archive_dir = temp_dir.join("archives");
    config.retention_batch_rows = 2;
    config.invocation_archive_ttl_days = 365;
    fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
    fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
    (pool, config, temp_dir)
}

#[tokio::test]
pub(crate) async fn retention_file_fixture_copies_current_schema_template_without_leaking_mutations()
 {
    let template_dir = make_temp_test_dir("retention-current-schema-template");
    let template_path = template_dir.join("current-schema.db");
    write_stateful_schema_template(&template_path)
        .await
        .expect("build current-schema template");

    let (first_pool, _first_config, first_dir) = retention_test_pool_and_config_from_template(
        "retention-template-first",
        Some(&template_path),
    )
    .await;
    sqlx::query("CREATE TABLE retention_template_isolation (id INTEGER PRIMARY KEY)")
        .execute(&first_pool)
        .await
        .expect("mutate first template copy");
    first_pool.close().await;

    let (second_pool, _second_config, second_dir) = retention_test_pool_and_config_from_template(
        "retention-template-second",
        Some(&template_path),
    )
    .await;
    let leaked_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'retention_template_isolation'",
    )
    .fetch_one(&second_pool)
    .await
    .expect("inspect second template copy");
    assert_eq!(leaked_table_count, 0, "template copies must stay isolated");
    second_pool.close().await;

    cleanup_temp_test_dir(&first_dir);
    cleanup_temp_test_dir(&second_dir);
    cleanup_temp_test_dir(&template_dir);
}

#[tokio::test]
pub(crate) async fn retention_fresh_schema_fixture_reapplies_schema_without_pooled_ddl_races() {
    let (pool, _config, temp_dir) =
        retention_fresh_schema_test_pool_and_config("retention-fresh-schema-reapply").await;

    ensure_schema(&pool)
        .await
        .expect("reapply fresh retention schema");

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

pub(crate) async fn retention_memory_test_pool_and_config(
    prefix: &str,
) -> (SqlitePool, AppConfig, PathBuf) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:{prefix}-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect retention in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("ensure retention in-memory schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
    config.archive_dir = temp_dir.join("archives");
    config.retention_batch_rows = 2;
    config.invocation_archive_ttl_days = 365;
    fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
    fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
    (pool, config, temp_dir)
}

pub(crate) fn cleanup_temp_test_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(unix)]
pub(crate) fn current_process_rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

pub(crate) fn shanghai_local_days_ago(days: i64, hour: u32, minute: u32, second: u32) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(naive)
}

pub(crate) fn shanghai_local_now_minus_secs(secs: i64) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai).naive_local();
    format_naive(now_local - ChronoDuration::seconds(secs))
}

pub(crate) fn utc_naive_from_shanghai_local_days_ago(
    days: i64,
    hour: u32,
    minute: u32,
    second: u32,
) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let local_naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(local_naive_to_utc(local_naive, Shanghai).naive_utc())
}

pub(crate) struct RetentionInvocationFixture<'a> {
    pub(crate) invoke_id: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) source: &'a str,
    pub(crate) status: &'a str,
    pub(crate) payload: Option<&'a str>,
    pub(crate) raw_response: &'a str,
    pub(crate) request_raw_path: Option<&'a Path>,
    pub(crate) response_raw_path: Option<&'a Path>,
    pub(crate) total_tokens: Option<i64>,
    pub(crate) cost: Option<f64>,
}

pub(crate) async fn insert_retention_invocation_with_fixture(
    pool: &SqlitePool,
    fixture: RetentionInvocationFixture<'_>,
) {
    let RetentionInvocationFixture {
        invoke_id,
        occurred_at,
        source,
        status,
        payload,
        raw_response,
        request_raw_path,
        response_raw_path,
        total_tokens,
        cost,
    } = fixture;
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            reasoning_tokens,
            total_tokens,
            cost,
            status,
            payload,
            raw_response,
            request_raw_path,
            request_raw_codec,
            request_raw_size,
            response_raw_path,
            response_raw_codec,
            response_raw_size
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(source)
    .bind(Some("gpt-5.2-codex"))
    .bind(Some(12_i64))
    .bind(Some(3_i64))
    .bind(Some(0_i64))
    .bind(Some(0_i64))
    .bind(total_tokens)
    .bind(cost)
    .bind(status)
    .bind(payload)
    .bind(raw_response)
    .bind(request_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(raw_codec_from_path(
        request_raw_path
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
    ))
    .bind(
        request_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .bind(response_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(raw_codec_from_path(
        response_raw_path
            .map(|path| path.to_string_lossy().to_string())
            .as_deref(),
    ))
    .bind(
        response_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .execute(pool)
    .await
    .expect("insert retention invocation");
}

pub(crate) async fn insert_retention_invocation(
    pool: &SqlitePool,
    fixture: RetentionInvocationFixture<'_>,
) {
    insert_retention_invocation_with_fixture(pool, fixture).await;
}

pub(crate) struct RetentionPoolAttemptFixture<'a> {
    pub(crate) invoke_id: &'a str,
    pub(crate) occurred_at: &'a str,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) attempt_index: i64,
    pub(crate) distinct_account_index: i64,
    pub(crate) same_account_retry_index: i64,
    pub(crate) status: &'a str,
    pub(crate) http_status: Option<i64>,
    pub(crate) failure_kind: Option<&'a str>,
    pub(crate) started_at: Option<&'a str>,
    pub(crate) finished_at: Option<&'a str>,
}

async fn insert_retention_pool_upstream_request_attempt_with_fixture(
    pool: &SqlitePool,
    fixture: RetentionPoolAttemptFixture<'_>,
) {
    let RetentionPoolAttemptFixture {
        invoke_id,
        occurred_at,
        upstream_account_id,
        attempt_index,
        distinct_account_index,
        same_account_retry_index,
        status,
        http_status,
        failure_kind,
        started_at,
        finished_at,
    } = fixture;
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id,
            occurred_at,
            endpoint,
            route_mode,
            sticky_key,
            upstream_account_id,
            upstream_route_key,
            attempt_index,
            distinct_account_index,
            same_account_retry_index,
            requester_ip,
            started_at,
            finished_at,
            status,
            http_status,
            failure_kind,
            error_message,
            connect_latency_ms,
            first_byte_latency_ms,
            stream_latency_ms,
            upstream_request_id
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
        )
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind("/v1/responses")
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(Some("sticky-retention"))
    .bind(upstream_account_id)
    .bind(None::<String>)
    .bind(attempt_index)
    .bind(distinct_account_index)
    .bind(same_account_retry_index)
    .bind(Some("203.0.113.1"))
    .bind(started_at)
    .bind(finished_at)
    .bind(status)
    .bind(http_status)
    .bind(failure_kind)
    .bind(Some("retention test"))
    .bind(Some(12.5_f64))
    .bind(Some(6.2_f64))
    .bind(Some(30.0_f64))
    .bind(Some("req_retention"))
    .execute(pool)
    .await
    .expect("insert retention pool attempt");
}

pub(crate) async fn insert_retention_pool_upstream_request_attempt(
    pool: &SqlitePool,
    fixture: RetentionPoolAttemptFixture<'_>,
) {
    insert_retention_pool_upstream_request_attempt_with_fixture(pool, fixture).await;
}

async fn legacy_raw_codec_schema_fixture() -> SqlitePool {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:ensure-schema-raw-codecs-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&db_url)
        .await
        .expect("connect schema sqlite");
    create_legacy_invocation_codec_fixture(&pool).await;
    create_legacy_archive_manifest_fixture(&pool).await;
    pool
}

async fn create_legacy_invocation_codec_fixture(pool: &SqlitePool) {
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
    .execute(pool)
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
    .execute(pool)
    .await
    .expect("insert legacy codec row");
}

async fn create_legacy_archive_manifest_fixture(pool: &SqlitePool) {
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
    .execute(pool)
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
    .execute(pool)
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
    .execute(pool)
    .await
    .expect("insert legacy archive manifest with text coverage bounds");
}

#[tokio::test]
pub(crate) async fn ensure_schema_backfills_raw_codecs_and_manifest_tables() {
    let pool = legacy_raw_codec_schema_fixture().await;

    ensure_schema(&pool).await.expect("ensure schema migration");
    assert_raw_codec_schema_migration(&pool).await;
    assert_raw_codec_schema_migration_is_idempotent(&pool).await;
    assert_archive_manifest_schema_migration(&pool).await;
}

async fn assert_raw_codec_schema_migration(pool: &SqlitePool) {
    let row = sqlx::query(
        "SELECT request_raw_codec, response_raw_codec FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("legacy-codec-row")
    .fetch_one(pool)
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
    .fetch_one(pool)
    .await
    .expect("load raw codec migration marker");
    assert_eq!(completed_migrations, 1);

    let completed_plan: Vec<String> = sqlx::query(
        "EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM codex_invocation_raw_codec_migrations WHERE migration_name = 'backfill_raw_codecs_v1')",
    )
    .fetch_all(pool)
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
}

async fn assert_raw_codec_schema_migration_is_idempotent(pool: &SqlitePool) {
    sqlx::query("UPDATE codex_invocations SET response_raw_codec = ?1 WHERE invoke_id = ?2")
        .bind(RAW_CODEC_IDENTITY)
        .bind("legacy-codec-row")
        .execute(pool)
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
    .execute(pool)
    .await
    .expect("install completed migration update guard");
    ensure_schema(pool)
        .await
        .expect("completed codec migration should be idempotent");
    let response_codec: String =
        sqlx::query_scalar("SELECT response_raw_codec FROM codex_invocations WHERE invoke_id = ?1")
            .bind("legacy-codec-row")
            .fetch_one(pool)
            .await
            .expect("load idempotent codec row");
    assert_eq!(response_codec, RAW_CODEC_IDENTITY);
}

async fn assert_archive_manifest_schema_migration(pool: &SqlitePool) {
    let archive_batch_columns = load_sqlite_table_columns(pool, "archive_batches")
        .await
        .expect("load archive batch columns");
    assert!(archive_batch_columns.contains("upstream_activity_manifest_refreshed_at"));
    assert!(archive_batch_columns.contains("coverage_start_epoch"));
    assert!(archive_batch_columns.contains("coverage_end_epoch"));
    sqlx::query(
        "UPDATE archive_batches SET coverage_start_at = '2026-03-01 08:00:00', coverage_end_at = '2026-03-01T01:00:00Z' WHERE file_path = '/tmp/legacy-coverage.sqlite.gz'",
    )
    .execute(pool)
    .await
    .expect("update archive coverage bounds through epoch trigger");
    let normalized_coverage = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT coverage_start_epoch, coverage_end_epoch FROM archive_batches WHERE file_path = '/tmp/legacy-coverage.sqlite.gz'",
    )
    .fetch_one(pool)
    .await
    .expect("load normalized archive coverage");
    assert_eq!(normalized_coverage.0, Some(1_772_323_200));
    assert_eq!(normalized_coverage.1, Some(1_772_326_800));
    let backfilled_coverage = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT coverage_start_epoch, coverage_end_epoch FROM archive_batches WHERE file_path = '/tmp/legacy-coverage-backfill.sqlite.gz'",
    )
    .fetch_one(pool)
    .await
    .expect("load backfilled archive coverage");
    assert_eq!(
        backfilled_coverage,
        (Some(1_772_323_200), Some(1_772_326_800))
    );
    let coverage_index: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'idx_archive_batches_invocation_coverage_epoch'",
    )
    .fetch_optional(pool)
    .await
    .expect("load archive coverage index");
    assert_eq!(
        coverage_index.as_deref(),
        Some("idx_archive_batches_invocation_coverage_epoch")
    );
    let manifest_columns = load_sqlite_table_columns(pool, "archive_batch_upstream_activity")
        .await
        .expect("load manifest columns");
    assert!(manifest_columns.contains("archive_batch_id"));
    assert!(manifest_columns.contains("account_id"));
    assert!(manifest_columns.contains("last_activity_at"));
}

#[tokio::test]
pub(crate) async fn ensure_schema_commits_raw_codec_backfill_and_marker_atomically() {
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
pub(crate) async fn ensure_schema_honors_legacy_raw_blob_seed_marker_for_raw_codecs() {
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
pub(crate) async fn raw_compression_budget_stops_after_first_batch_when_budget_is_exhausted() {
    let (pool, mut config, temp_dir) =
        retention_test_pool_and_config("retention-catchup-budget").await;
    config.proxy_raw_hot_secs = 60;
    config.proxy_raw_compression = RawCompressionCodec::Gzip;
    config.retention_batch_rows = 1;

    for (invoke_id, hour, file_name) in [
        ("budget-oldest", 8, "budget-oldest.bin"),
        ("budget-middle", 9, "budget-middle.bin"),
        ("budget-newest", 10, "budget-newest.bin"),
    ] {
        let raw_path = config.proxy_raw_dir.join(file_name);
        fs::write(&raw_path, invoke_id.as_bytes()).expect("write budget raw file");
        insert_retention_invocation_with_fixture(
            &pool,
            RetentionInvocationFixture {
                invoke_id,
                occurred_at: &shanghai_local_days_ago(2, hour, 0, 0),
                source: SOURCE_PROXY,
                status: "failed",
                payload: Some("{\"endpoint\":\"/v1/responses\"}"),
                raw_response: "{\"ok\":false}",
                request_raw_path: Some(&raw_path),
                response_raw_path: None,
                total_tokens: Some(10),
                cost: Some(0.01),
            },
        )
        .await;
    }

    let first_pass = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        Some(Duration::ZERO),
    )
    .await
    .expect("run raw compression with zero catchup budget");
    assert_eq!(first_pass.files_considered, 1);
    assert_eq!(first_pass.files_compressed, 1);

    let remaining_after_first: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_codec = 'identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining raw backlog after first pass");
    assert_eq!(remaining_after_first, 2);

    let catchup = compress_cold_proxy_raw_payloads_with_budget(
        &pool,
        &config,
        config.database_path.parent(),
        false,
        None,
    )
    .await
    .expect("run unrestricted raw compression catchup");
    assert_eq!(catchup.files_considered, 2);
    assert_eq!(catchup.files_compressed, 2);

    let remaining_after_catchup: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM codex_invocations WHERE request_raw_codec = 'identity'",
    )
    .fetch_one(&pool)
    .await
    .expect("count remaining raw backlog after catchup");
    assert_eq!(remaining_after_catchup, 0);

    cleanup_temp_test_dir(&temp_dir);
}

use super::*;
