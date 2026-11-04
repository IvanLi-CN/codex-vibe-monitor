use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
#[allow(unused_imports)]
use serde_json::json;
use sqlx::SqlitePool;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};
use tokio::{net::TcpListener, sync::Mutex, time::timeout};

mod archive_compat_and_query_filters;
mod imported_oauth_normalization_and_validation;
mod sync_jobs_and_group_policy;
mod usage_snapshot_and_oauth_callbacks;

pub(crate) use archive_compat_and_query_filters::*;
pub(crate) use usage_snapshot_and_oauth_callbacks::*;

pub(crate) async fn test_pool() -> SqlitePool {
    if std::env::var_os(crate::tests::STATEFUL_SCHEMA_TEMPLATE_PATH_ENV).is_some() {
        return crate::tests::test_current_schema_pool().await;
    }

    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    crate::ensure_schema(&pool).await.expect("ensure schema");
    pool
}

pub(crate) fn test_required_group_bound_proxy_keys() -> Vec<String> {
    vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
}

pub(crate) fn test_required_group_name() -> &'static str {
    "test-direct-group"
}

pub(crate) async fn upsert_test_group_binding(
    pool: &SqlitePool,
    group_name: &str,
    bound_proxy_keys: Vec<String>,
) {
    let now_iso = format_utc_iso(Utc::now());
    let bound_proxy_keys_json =
        encode_group_bound_proxy_keys_json(&bound_proxy_keys).expect("encode test bindings");
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name, note, bound_proxy_keys_json, created_at, updated_at
            ) VALUES (?1, '', ?2, ?3, ?3)
            ON CONFLICT(group_name) DO UPDATE SET
                bound_proxy_keys_json = excluded.bound_proxy_keys_json,
                updated_at = excluded.updated_at
            "#,
    )
    .bind(group_name)
    .bind(bound_proxy_keys_json)
    .bind(&now_iso)
    .execute(pool)
    .await
    .expect("upsert test group binding");
}

pub(crate) async fn ensure_test_group_binding(pool: &SqlitePool, group_name: &str) {
    upsert_test_group_binding(pool, group_name, test_required_group_bound_proxy_keys()).await;
}

pub(crate) async fn set_test_account_group_name(
    pool: &SqlitePool,
    account_id: i64,
    group_name: Option<&str>,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(group_name)
    .bind(&now_iso)
    .execute(pool)
    .await
    .expect("set test account group name");
}

pub(crate) async fn set_test_account_token_expires_at(
    pool: &SqlitePool,
    account_id: i64,
    token_expires_at: &str,
) {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET token_expires_at = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind(token_expires_at)
    .bind(&now_iso)
    .execute(pool)
    .await
    .expect("set test account token expires at");
}

pub(crate) async fn test_app_state_with_usage_base(base_url: &str) -> Arc<AppState> {
    test_app_state_with_usage_base_and_parallelism(
        base_url,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await
}

pub(crate) async fn test_app_state_with_usage_base_and_parallelism(
    base_url: &str,
    maintenance_parallelism: usize,
) -> Arc<AppState> {
    test_app_state_with_upstream_endpoints_and_parallelism(
        base_url,
        DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER,
        "codex-vibe-monitor/test",
        maintenance_parallelism,
    )
    .await
}

pub(crate) async fn test_app_state_with_usage_and_oauth_base(
    usage_base_url: &str,
    oauth_issuer: &str,
) -> Arc<AppState> {
    test_app_state_with_upstream_endpoints_and_parallelism(
        usage_base_url,
        oauth_issuer,
        UPSTREAM_USAGE_BROWSER_USER_AGENT,
        DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM,
    )
    .await
}

async fn test_app_state_with_upstream_endpoints_and_parallelism(
    usage_base_url: &str,
    oauth_issuer: &str,
    user_agent: &str,
    maintenance_parallelism: usize,
) -> Arc<AppState> {
    let mut config = usage_snapshot_test_config(usage_base_url, user_agent);
    config.upstream_accounts_oauth_issuer = Url::parse(oauth_issuer).expect("valid oauth issuer");
    test_app_state_with_config_and_parallelism(config, maintenance_parallelism).await
}

pub(crate) async fn test_app_state_with_config_and_parallelism(
    config: AppConfig,
    maintenance_parallelism: usize,
) -> Arc<AppState> {
    let http_clients = HttpClients::build(&config).expect("build http clients");
    let (broadcaster, _) = broadcast::channel(8);
    let raw_capture_root = config.resolved_proxy_raw_dir();
    let pool = test_pool().await;
    Arc::new(AppState {
        system_storage: Arc::new(SystemStorageRuntime::new(&config)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit(&config))),
        config,
        pg_analytics: Arc::new(PgAnalyticsRuntime::disabled()),
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
        proxy_runtime_attempts: Arc::new(RuntimeAttemptStore::default()),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        subscription_hub: Arc::new(crate::SubscriptionHub::new()),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache { quota: None })),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
        process_started_at_utc: chrono::Utc::now(),
        observability: ObservabilityRuntime::disabled_for_tests(),
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore: Arc::new(Semaphore::new(4)),
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        raw_capture_circuit: Arc::new(RawCaptureCircuitBreaker::new(raw_capture_root)),
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            "xray".to_string(),
            PathBuf::from("target/xray-supervisor-tests"),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(PromptCacheConversationsCacheState {
            entries: HashMap::new(),
            in_flight: HashMap::new(),
            generation: 0,
            identity_cache: PromptCacheConversationIdentityCache::default(),
        })),
        dashboard_activity_snapshot_cache: Arc::new(Mutex::new(
            DashboardActivitySnapshotCacheState::default(),
        )),
        terminal_projection_hub: Arc::new(crate::TerminalProjectionHub::default()),
        long_term_projection_runtime: Arc::new(Mutex::new(
            crate::LongTermProjectionRuntime::default(),
        )),
        memory_diagnostics: Arc::new(crate::MemoryDiagnosticsRuntime::default()),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_availability: PoolRoutingAvailabilitySignal::default(),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_routing_test_data_version_connection: Arc::new(Mutex::new(None)),
        pool_model_routing_cache_write_lock: Arc::new(Mutex::new(())),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
        fallback_proxy_429_retry_delay_override: None,
        pool_no_available_wait: PoolNoAvailableWaitSettings::default(),
        hourly_rollup_sync_lock: Arc::new(Mutex::new(())),
        upstream_accounts: Arc::new(
            UpstreamAccountsRuntime::test_instance_with_maintenance_parallelism(
                maintenance_parallelism,
            ),
        ),
    })
}

pub(crate) async fn ensure_window_actual_usage_test_tables(pool: &SqlitePool) {
    sqlx::query(&codex_invocations_create_sql("codex_invocations"))
        .execute(pool)
        .await
        .expect("create codex_invocations table");
    sqlx::query(
        r#"
            CREATE TABLE IF NOT EXISTS archive_batches (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                dataset TEXT NOT NULL,
                month_key TEXT NOT NULL,
                day_key TEXT,
                part_key TEXT,
                file_path TEXT NOT NULL,
                sha256 TEXT,
                status TEXT NOT NULL,
                coverage_start_at TEXT,
                coverage_end_at TEXT,
                created_at TEXT NOT NULL
            )
            "#,
    )
    .execute(pool)
    .await
    .expect("create archive_batches table");
    let _ = sqlx::query("ALTER TABLE archive_batches ADD COLUMN sha256 TEXT")
        .execute(pool)
        .await;
}

pub(crate) fn shanghai_local_iso(timestamp: DateTime<Utc>) -> String {
    format_naive(timestamp.with_timezone(&Shanghai).naive_local())
}

pub(crate) async fn insert_window_actual_usage_invocation(
    pool: &SqlitePool,
    account_id: i64,
    occurred_at: &str,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_input_tokens: Option<i64>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
) {
    sqlx::query(
        r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                input_tokens,
                output_tokens,
                cache_input_tokens,
                total_tokens,
                cost,
                status,
                payload,
                raw_response,
                created_at
            ) VALUES (
                ?1,
                ?2,
                'test',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                'completed',
                ?8,
                '{}',
                ?2
            )
            "#,
    )
    .bind(format!("invoke-{}", random_base36(10).expect("invoke id")))
    .bind(occurred_at)
    .bind(input_tokens)
    .bind(output_tokens)
    .bind(cache_input_tokens)
    .bind(total_tokens)
    .bind(cost)
    .bind(json!({ "upstreamAccountId": account_id }).to_string())
    .execute(pool)
    .await
    .expect("insert codex_invocations row");
}

pub(crate) async fn seed_window_actual_usage_archive_batch(
    pool: &SqlitePool,
    archive_dir: &Path,
    batch_name: &str,
    rows: &[(
        i64,
        String,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<f64>,
    )],
) -> PathBuf {
    std::fs::create_dir_all(archive_dir).expect("create archive dir");
    let archive_db_path = archive_dir.join(format!("{batch_name}.sqlite"));
    let archive_gzip_path = archive_dir.join(format!("{batch_name}.sqlite.gz"));
    let _ = std::fs::remove_file(&archive_db_path);
    let _ = std::fs::remove_file(&archive_gzip_path);
    std::fs::File::create(&archive_db_path).expect("create archive sqlite");

    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open archive sqlite");
    let create_sql = CODEX_INVOCATIONS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive codex_invocations");

    for (index, row) in rows.iter().enumerate() {
        sqlx::query(
            r#"
                INSERT INTO codex_invocations (
                    id,
                    invoke_id,
                    occurred_at,
                    source,
                    input_tokens,
                    output_tokens,
                    cache_input_tokens,
                    total_tokens,
                    cost,
                    status,
                    payload,
                    raw_response,
                    created_at
                ) VALUES (
                    ?1,
                    ?2,
                    ?3,
                    'test',
                    ?4,
                    ?5,
                    ?6,
                    ?7,
                    ?8,
                    'completed',
                    ?9,
                    '{}',
                    ?3
                )
                "#,
        )
        .bind(index as i64 + 1)
        .bind(format!(
            "archived-invoke-{}",
            random_base36(10).expect("archive invoke id")
        ))
        .bind(&row.1)
        .bind(row.2)
        .bind(row.3)
        .bind(row.4)
        .bind(row.5)
        .bind(row.6)
        .bind(json!({ "upstreamAccountId": row.0 }).to_string())
        .execute(&archive_pool)
        .await
        .expect("insert archive codex_invocations row");
    }

    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_gzip_path)
        .expect("compress archive sqlite");

    let coverage_start_at = rows
        .iter()
        .map(|row| row.1.as_str())
        .min()
        .expect("archive coverage start");
    let coverage_end_at = rows
        .iter()
        .map(|row| row.1.as_str())
        .max()
        .expect("archive coverage end");
    let month_key = &coverage_start_at[..7];
    let day_key = &coverage_start_at[..10];

    sqlx::query(
        r#"
            INSERT INTO archive_batches (
                dataset,
                month_key,
                day_key,
                part_key,
                file_path,
                sha256,
                row_count,
                status,
                coverage_start_at,
                coverage_end_at,
                created_at
            ) VALUES (
                'codex_invocations',
                ?1,
                ?2,
                'part-000',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                ?8,
                ?9
            )
            "#,
    )
    .bind(month_key)
    .bind(day_key)
    .bind(archive_gzip_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_gzip_path).expect("archive sha256"))
    .bind(rows.len() as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .bind(coverage_end_at)
    .execute(pool)
    .await
    .expect("insert archive batch manifest");

    archive_gzip_path
}

pub(crate) fn assert_cost_close(actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff < 1e-9,
        "expected {expected}, got {actual}, diff={diff}"
    );
}

#[derive(Clone)]
pub(crate) struct KaisouMailStubState {
    pub(crate) domains: Vec<String>,
    pub(crate) emails: Arc<Mutex<Vec<(String, String, Option<String>)>>>,
    pub(crate) create_requests: Arc<Mutex<Vec<Value>>>,
    pub(crate) generated_requests: Arc<Mutex<Vec<(String, String)>>>,
    pub(crate) deleted_ids: Arc<Mutex<Vec<String>>>,
    pub(crate) next_generated_id: Arc<AtomicUsize>,
}

pub(crate) struct KaisouMailTestHarness {
    pub(crate) state: Arc<AppState>,
    pub(crate) stub: KaisouMailStubState,
    pub(crate) server: tokio::task::JoinHandle<()>,
}

impl KaisouMailTestHarness {
    pub(crate) fn abort(self) {
        self.server.abort();
    }
}
