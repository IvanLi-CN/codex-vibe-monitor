use super::*;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use sqlx::SqlitePool;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicUsize},
    time::Duration,
};
use tokio::{net::TcpListener, time::timeout};

type GroupScopedArchiveRow<'a> = (&'a str, &'a str, Option<&'a str>, Option<&'a str>, &'a str);

pub(crate) async fn wait_for_imported_oauth_validation_job_terminal(
    job: &Arc<ImportedOauthValidationJob>,
) -> ImportedOauthValidationTerminalEvent {
    timeout(Duration::from_secs(15), async {
        loop {
            if let Some(terminal) = job.terminal_event.lock().await.clone() {
                return terminal;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("validation job should finish within timeout")
}

pub(crate) fn test_summary_with_statuses(
    work_status: &str,
    enable_status: &str,
    health_status: &str,
    sync_state: &str,
) -> UpstreamAccountSummary {
    UpstreamAccountSummary {
        id: 1,
        kind: UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX.to_string(),
        provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
        display_name: "Test account".to_string(),
        group_name: Some("alpha".to_string()),
        is_mother: false,
        status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
        display_status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
        enabled: enable_status == UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        work_status: work_status.to_string(),
        enable_status: enable_status.to_string(),
        health_status: health_status.to_string(),
        sync_state: sync_state.to_string(),
        routing_block_reason_code: None,
        routing_block_reason_message: None,
        routing_block_until: None,
        email: Some("tester@example.com".to_string()),
        chatgpt_account_id: Some("acct_test".to_string()),
        plan_type: Some("pro".to_string()),
        masked_api_key: None,
        has_refresh_token: true,
        last_synced_at: None,
        last_successful_sync_at: None,
        last_activity_at: None,
        active_conversation_count: 0,
        last_error: None,
        last_error_at: None,
        last_action: None,
        last_action_source: None,
        last_action_reason_code: None,
        last_action_reason_message: None,
        last_action_http_status: None,
        last_action_invoke_id: None,
        last_action_at: None,
        cooldown_until: None,
        bound_proxy_keys: Vec::new(),
        current_forward_proxy_key: None,
        current_forward_proxy_display_name: None,
        current_forward_proxy_state: UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED.to_string(),
        token_expires_at: None,
        primary_window: None,
        secondary_window: None,
        credits: None,
        local_limits: None,
        compact_support: CompactSupportState {
            status: "unknown".to_string(),
            observed_at: None,
            reason: None,
        },
        duplicate_info: None,
        tags: vec![],
        effective_routing_rule: test_effective_routing_rule(),
        response_endpoint_capability: unknown_capability_state(),
        chat_completions_capability: unknown_capability_state(),
        image_endpoint_capability: unknown_capability_state(),
        response_image_tool_capability: unknown_capability_state(),
        codex_imagegen_capability: unknown_capability_state(),
        standalone_search_capability: unknown_capability_state(),
    }
}

fn unknown_capability_state() -> UpstreamCapabilityState {
    UpstreamCapabilityState {
        observed: CapabilitySupport::Unknown,
        override_value: None,
        effective: CapabilitySupport::Unknown,
        observed_at: None,
        reason: None,
    }
}

fn test_effective_routing_rule() -> EffectiveRoutingRule {
    EffectiveRoutingRule {
        allow_cut_out: true,
        allow_cut_in: true,
        priority_tier: TagPriorityTier::Normal,
        fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
        image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
        codex_imagegen_rewrite_mode: Default::default(),
        request_compression_algorithm: RequestCompressionAlgorithm::Identity,
        concurrency_limit: 0,
        upstream_429_retry_enabled: false,
        upstream_429_max_retries: 0,
        available_models: vec![],
        available_models_mode: AvailableModelsMode::Allowlist,
        available_models_defined: false,
        tag_available_models: None,
        status_change_reasons: default_status_change_reasons(),
        status_change_reason_field_sources: default_status_change_reason_field_sources("root"),
        system_denied_models: vec![],
        source_tag_ids: vec![],
        source_tag_names: vec![],
        field_sources: EffectiveRoutingRuleFieldSources {
            allow_cut_out: "root".to_string(),
            allow_cut_in: "root".to_string(),
            priority_tier: "root".to_string(),
            fast_mode_rewrite_mode: "root".to_string(),
            image_tool_rewrite_mode: "root".to_string(),
            codex_imagegen_rewrite_mode: "root".to_string(),
            request_compression_algorithm: "root".to_string(),
            concurrency_limit: "root".to_string(),
            upstream_429_retry: "root".to_string(),
            available_models: "root".to_string(),
            available_models_mode: "root".to_string(),
            system_denied_models: "root".to_string(),
        },
        timeouts: RoutingTimeoutSettings {
            responses_first_byte_timeout_secs: Some(120),
            compact_first_byte_timeout_secs: Some(300),
            image_first_byte_timeout_secs: Some(300),
            responses_stream_timeout_secs: Some(300),
            compact_stream_timeout_secs: Some(300),
        },
        timeout_field_sources: RoutingTimeoutFieldSources {
            responses_first_byte_timeout_secs: "root".to_string(),
            compact_first_byte_timeout_secs: "root".to_string(),
            image_first_byte_timeout_secs: "root".to_string(),
            responses_stream_timeout_secs: "root".to_string(),
            compact_stream_timeout_secs: "root".to_string(),
        },
    }
}

pub(crate) async fn seed_group_scoped_pool_attempt(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    group_name_snapshot: Option<&str>,
    proxy_binding_key_snapshot: Option<&str>,
    status: &str,
) {
    let phase = if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    };
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_request_attempts (
                invoke_id,
                occurred_at,
                endpoint,
                route_mode,
                sticky_key,
                group_name_snapshot,
                proxy_binding_key_snapshot,
                upstream_account_id,
                upstream_route_key,
                attempt_index,
                distinct_account_index,
                same_account_retry_index,
                requester_ip,
                started_at,
                finished_at,
                status,
                phase,
                http_status,
                error_message,
                created_at
            )
            VALUES (
                ?1, ?2, '/v1/responses', ?3, 'sticky-group-scope', ?4, ?5, 41, 'route-group-scope',
                1, 1, 0, '203.0.113.9', ?2, ?2, ?6, ?7, ?8, ?9, datetime('now')
            )
            "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(INVOCATION_ROUTE_MODE_POOL)
    .bind(group_name_snapshot)
    .bind(proxy_binding_key_snapshot)
    .bind(status)
    .bind(phase)
    .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
    .bind(
        (status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some("group-scoped failure"),
    )
    .execute(pool)
    .await
    .expect("seed group-scoped pool attempt");
}

pub(crate) async fn seed_pool_upstream_attempt_at(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: DateTime<Utc>,
    proxy_binding_key_snapshot: Option<&str>,
    status: &str,
) {
    let occurred_at = format_naive(occurred_at.with_timezone(&Shanghai).naive_local());
    let phase = if status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
    };
    sqlx::query(
            r#"
            INSERT INTO pool_upstream_request_attempts (
                invoke_id,
                occurred_at,
                endpoint,
                route_mode,
                sticky_key,
                group_name_snapshot,
                proxy_binding_key_snapshot,
                upstream_account_id,
                upstream_route_key,
                attempt_index,
                distinct_account_index,
                same_account_retry_index,
                requester_ip,
                started_at,
                finished_at,
                status,
                phase,
                http_status,
                error_message,
                connect_latency_ms,
                first_byte_latency_ms,
                stream_latency_ms,
                created_at
            )
            VALUES (
                ?1, ?2, '/v1/responses', ?3, 'sticky-binding-nodes', NULL, ?4, 41, 'route-binding-nodes',
                1, 1, 0, '203.0.113.9', ?2, ?2, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now')
            )
            "#,
        )
        .bind(invoke_id)
        .bind(&occurred_at)
        .bind(INVOCATION_ROUTE_MODE_POOL)
        .bind(proxy_binding_key_snapshot)
        .bind(status)
        .bind(phase)
        .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
        .bind(
            (status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
                .then_some("binding node failure"),
        )
        .bind((status != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(180.0))
        .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(120.0))
        .bind((status == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(320.0))
        .execute(pool)
        .await
        .expect("seed pool upstream attempt");
}

pub(crate) async fn seed_forward_proxy_metadata_history(
    pool: &SqlitePool,
    proxy_key: &str,
    display_name: &str,
    source: &str,
    endpoint_url: &str,
) {
    sqlx::query(
        r#"
            INSERT INTO forward_proxy_metadata_history (
                proxy_key,
                display_name,
                source,
                endpoint_url,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, datetime('now'))
            ON CONFLICT(proxy_key) DO UPDATE SET
                display_name = excluded.display_name,
                source = excluded.source,
                endpoint_url = excluded.endpoint_url,
                updated_at = datetime('now')
            "#,
    )
    .bind(proxy_key)
    .bind(display_name)
    .bind(source)
    .bind(endpoint_url)
    .execute(pool)
    .await
    .expect("seed forward proxy metadata history");
}

pub(crate) async fn seed_group_scoped_pool_attempt_archive_batch(
    pool: &SqlitePool,
    archive_dir: &Path,
    batch_name: &str,
    rows: &[GroupScopedArchiveRow<'_>],
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
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL.replace("archive_db.", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create archive pool attempt schema");

    insert_group_scoped_archive_rows(&archive_pool, rows).await;

    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_gzip_path)
        .expect("compress archive sqlite");
    let archive_manifest_path =
        std::fs::canonicalize(&archive_gzip_path).expect("canonicalize archive gzip");

    let coverage_start_at = rows
        .iter()
        .map(|row| row.1)
        .min()
        .expect("archive coverage start");
    let coverage_end_at = rows
        .iter()
        .map(|row| row.1)
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
            )
            VALUES (
                'pool_upstream_request_attempts',
                ?1,
                ?2,
                'part-000',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                ?8,
                datetime('now')
            )
            "#,
    )
    .bind(month_key)
    .bind(day_key)
    .bind(archive_manifest_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_gzip_path).expect("archive sha256"))
    .bind(rows.len() as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .execute(pool)
    .await
    .expect("insert pool attempt archive batch manifest");

    archive_gzip_path
}

async fn insert_group_scoped_archive_rows(
    archive_pool: &SqlitePool,
    rows: &[GroupScopedArchiveRow<'_>],
) {
    for (index, row) in rows.iter().enumerate() {
        let phase = if row.4 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
        } else {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
        };
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, group_name_snapshot, proxy_binding_key_snapshot, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, finished_at, status, phase, http_status, error_message, created_at) VALUES (?1, ?2, ?3, '/v1/responses', ?4, 'archived-group-scope', ?5, ?6, 41, 'archived-route-group-scope', 1, 1, 0, '203.0.113.19', ?3, ?3, ?7, ?8, ?9, ?10, datetime('now'))",
        )
        .bind(10_000_i64 + index as i64)
        .bind(row.0)
        .bind(row.1)
        .bind(INVOCATION_ROUTE_MODE_POOL)
        .bind(row.2)
        .bind(row.3)
        .bind(row.4)
        .bind(phase)
        .bind((row.4 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
        .bind((row.4 != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some("group-scoped archived failure"))
        .execute(archive_pool)
        .await
        .expect("insert archive group-scoped pool attempt row");
    }
}

pub(crate) async fn seed_legacy_group_scoped_pool_attempt_archive_batch_without_scope_columns(
    pool: &SqlitePool,
    archive_dir: &Path,
    batch_name: &str,
    rows: &[(&str, &str, &str)],
) -> PathBuf {
    std::fs::create_dir_all(archive_dir).expect("create legacy archive dir");
    let archive_db_path = archive_dir.join(format!("{batch_name}.sqlite"));
    let archive_gzip_path = archive_dir.join(format!("{batch_name}.sqlite.gz"));
    let _ = std::fs::remove_file(&archive_db_path);
    let _ = std::fs::remove_file(&archive_gzip_path);
    std::fs::File::create(&archive_db_path).expect("create legacy archive sqlite");

    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open legacy archive sqlite");
    let create_sql = POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_CREATE_SQL
        .replace("archive_db.", "")
        .replace("    group_name_snapshot TEXT,\n", "")
        .replace("    proxy_binding_key_snapshot TEXT,\n", "");
    sqlx::query(&create_sql)
        .execute(&archive_pool)
        .await
        .expect("create legacy archive pool attempt schema");

    insert_legacy_archive_rows(&archive_pool, rows).await;

    archive_pool.close().await;
    deflate_sqlite_file_to_gzip(&archive_db_path, &archive_gzip_path)
        .expect("compress legacy archive sqlite");
    let archive_manifest_path =
        std::fs::canonicalize(&archive_gzip_path).expect("canonicalize legacy archive gzip");

    let coverage_start_at = rows
        .iter()
        .map(|row| row.1)
        .min()
        .expect("legacy archive coverage start");
    let coverage_end_at = rows
        .iter()
        .map(|row| row.1)
        .max()
        .expect("legacy archive coverage end");
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
            )
            VALUES (
                'pool_upstream_request_attempts',
                ?1,
                ?2,
                'part-legacy-000',
                ?3,
                ?4,
                ?5,
                ?6,
                ?7,
                ?8,
                datetime('now')
            )
            "#,
    )
    .bind(month_key)
    .bind(day_key)
    .bind(archive_manifest_path.to_string_lossy().to_string())
    .bind(sha256_hex_file(&archive_gzip_path).expect("legacy archive sha256"))
    .bind(rows.len() as i64)
    .bind(ARCHIVE_STATUS_COMPLETED)
    .bind(coverage_start_at)
    .bind(coverage_end_at)
    .execute(pool)
    .await
    .expect("insert legacy pool attempt archive batch manifest");

    archive_gzip_path
}

async fn insert_legacy_archive_rows(archive_pool: &SqlitePool, rows: &[(&str, &str, &str)]) {
    for (index, row) in rows.iter().enumerate() {
        let phase = if row.2 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
        } else {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
        };
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts (id, invoke_id, occurred_at, endpoint, route_mode, sticky_key, upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, requester_ip, started_at, finished_at, status, phase, http_status, error_message, created_at) VALUES (?1, ?2, ?3, '/v1/responses', ?4, 'legacy-archived-group-scope', 41, 'legacy-archived-route-group-scope', 1, 1, 0, '203.0.113.29', ?3, ?3, ?5, ?6, ?7, ?8, datetime('now'))",
        )
        .bind(20_000_i64 + index as i64)
        .bind(row.0)
        .bind(row.1)
        .bind(INVOCATION_ROUTE_MODE_POOL)
        .bind(row.2)
        .bind(phase)
        .bind((row.2 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
        .bind((row.2 != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some("legacy archived failure"))
        .execute(archive_pool)
        .await
        .expect("insert legacy archive pool attempt row");
    }
}

#[test]
pub(crate) fn derive_secret_key_is_stable() {
    let lhs = derive_secret_key("alpha");
    let rhs = derive_secret_key("alpha");
    assert_eq!(lhs, rhs);
}

#[test]
pub(crate) fn credential_round_trip_works() {
    let key = derive_secret_key("top-secret");
    let encrypted = encrypt_credentials(
        &key,
        &StoredCredentials::ApiKey(StoredApiKeyCredentials {
            api_key: "sk-test-1234".to_string(),
        }),
    )
    .expect("encrypt credentials");
    let decrypted = decrypt_credentials(&key, &encrypted).expect("decrypt credentials");
    let StoredCredentials::ApiKey(value) = decrypted else {
        panic!("expected API key credentials")
    };
    assert_eq!(value.api_key, "sk-test-1234");
}

#[test]
pub(crate) fn deserialize_optional_field_distinguishes_missing_null_and_value() {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Payload {
        #[serde(default, deserialize_with = "deserialize_optional_field")]
        upstream_base_url: OptionalField<String>,
    }

    let missing: Payload = serde_json::from_value(json!({})).expect("deserialize missing");
    assert_eq!(missing.upstream_base_url, OptionalField::Missing);

    let null_value: Payload =
        serde_json::from_value(json!({ "upstreamBaseUrl": null })).expect("deserialize null");
    assert_eq!(null_value.upstream_base_url, OptionalField::Null);

    let string_value: Payload = serde_json::from_value(json!({
        "upstreamBaseUrl": "https://proxy.example.com/gateway"
    }))
    .expect("deserialize string");
    assert_eq!(
        string_value.upstream_base_url,
        OptionalField::Value("https://proxy.example.com/gateway".to_string())
    );
}

#[test]
pub(crate) fn list_query_deserializes_repeated_status_filters() {
    let query = parse_list_upstream_accounts_query(
            &"/api/pool/upstream-accounts?workStatus=working&workStatus=rate_limited&workStatus=unavailable&enableStatus=enabled&healthStatus=normal&healthStatus=needs_reauth"
                .parse()
                .expect("parse uri"),
        )
        .expect("deserialize repeated filters");

    assert_eq!(
        query.work_status,
        vec![
            UPSTREAM_ACCOUNT_WORK_STATUS_WORKING.to_string(),
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED.to_string(),
            UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE.to_string(),
        ]
    );
    assert_eq!(
        query.enable_status,
        vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED.to_string()]
    );
    assert_eq!(
        query.health_status,
        vec![
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string(),
            UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH.to_string(),
        ]
    );
}

#[test]
pub(crate) fn list_query_keeps_single_status_filter_compatible() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?workStatus=idle&enableStatus=disabled&healthStatus=normal"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize single filters");

    assert_eq!(
        query.work_status,
        vec![UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string()]
    );
    assert_eq!(
        query.enable_status,
        vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED.to_string()]
    );
    assert_eq!(
        query.health_status,
        vec![UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string()]
    );
}

#[test]
pub(crate) fn list_query_parses_include_all_flag() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?includeAll=1&page=3&pageSize=50"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize includeAll");

    assert_eq!(query.include_all, Some(true));
    assert_eq!(query.page, Some(3));
    assert_eq!(query.page_size, Some(50));
}

#[test]
pub(crate) fn list_query_parses_single_tag_filter() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?tagIds=5"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize single tag filter");

    assert_eq!(query.tag_ids, vec![5]);
}

#[test]
pub(crate) fn list_query_parses_repeated_tag_filters() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?tagIds=1&tagIds=2"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize repeated tag filters");

    assert_eq!(query.tag_ids, vec![1, 2]);
}

#[test]
pub(crate) fn list_query_rejects_invalid_tag_filter() {
    let err = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?tagIds=abc"
            .parse()
            .expect("parse uri"),
    )
    .expect_err("invalid tagIds should fail");

    assert!(err.contains("invalid tagIds value `abc`; expected integer"));
}

#[test]
pub(crate) fn list_query_parses_exact_group_filter() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?groupExact=production"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize exact group filter");

    assert_eq!(query.group_exact, vec!["production".to_string()]);
    assert_eq!(query.group_search, None);
}

#[test]
pub(crate) fn list_query_parses_repeated_exact_group_filters() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?groupExact=production&groupExact=staging"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize repeated exact group filters");

    assert_eq!(
        query.group_exact,
        vec!["production".to_string(), "staging".to_string()]
    );
    assert_eq!(query.group_search, None);
}

#[test]
pub(crate) fn list_query_rejects_invalid_include_all_flag() {
    let err = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?includeAll=maybe"
            .parse()
            .expect("parse uri"),
    )
    .expect_err("invalid includeAll should fail");

    assert!(err.contains("invalid includeAll value"));
}

#[test]
pub(crate) fn forward_proxy_binding_nodes_query_keeps_repeated_keys_and_include_current() {
    let query = parse_list_forward_proxy_binding_nodes_query(
            &"/api/pool/forward-proxy-binding-nodes?includeCurrent=true&groupName=prod&key=legacy-a&key=&key=legacy-b"
                .parse()
                .expect("parse uri"),
        )
        .expect("deserialize binding query");

    assert!(query.include_current);
    assert_eq!(query.group_name.as_deref(), Some("prod"));
    assert_eq!(
        query.key,
        vec![
            "legacy-a".to_string(),
            "".to_string(),
            "legacy-b".to_string(),
        ]
    );
}

#[test]
pub(crate) fn forward_proxy_binding_nodes_query_ignores_blank_group_name() {
    let query = parse_list_forward_proxy_binding_nodes_query(
        &"/api/pool/forward-proxy-binding-nodes?groupName=%20%20&includeCurrent=true"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize blank group binding query");

    assert!(query.include_current);
    assert_eq!(query.group_name, None);
}

#[tokio::test]
pub(crate) async fn list_forward_proxy_binding_nodes_returns_current_nodes_and_deduplicated_requested_missing_keys()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    crate::ensure_schema(&state.pool)
        .await
        .expect("ensure full schema for grouped binding stats");
    crate::ensure_schema(&state.pool)
        .await
        .expect("ensure full schema for forward proxy settings");
    let settings = ForwardProxySettings {
        proxy_urls: vec!["http://127.0.0.1:17890#JP Edge 01".to_string()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
    }

    let Json(nodes) = list_forward_proxy_binding_nodes(
            State(state),
            "/api/pool/forward-proxy-binding-nodes?includeCurrent=true&key=&key=legacy-missing-key&key=legacy-missing-key"
                .parse()
                .expect("parse uri"),
        )
        .await
        .expect("list forward proxy binding nodes");

    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.key == "legacy-missing-key")
            .count(),
        1
    );
    assert!(
        nodes
            .iter()
            .any(|node| node.key == "legacy-missing-key" && !node.selectable),
        "missing requested key should be returned once as an unavailable option"
    );
    assert!(
        nodes.iter().any(|node| node.selectable),
        "current selectable binding nodes should still be returned for dialog choices"
    );
}

struct ForwardProxyBindingFixture {
    state: Arc<AppState>,
    manual_key: String,
    manual_display_name: String,
    manual_bucket_epoch: i64,
}

async fn create_forward_proxy_binding_fixture() -> ForwardProxyBindingFixture {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    crate::ensure_schema(&state.pool)
        .await
        .expect("ensure full schema for forward proxy settings");
    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("persist forward proxy settings");
    let (manual_key, manual_display_name) = {
        let manager = state.forward_proxy.lock().await;
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| (node.key, node.display_name))
            .expect("manual binding key")
    };
    let range_end_epoch = align_bucket_epoch(Utc::now().timestamp(), 3600, 0) + 3600;
    ForwardProxyBindingFixture {
        state,
        manual_key,
        manual_display_name,
        manual_bucket_epoch: range_end_epoch - 24 * 3600 + 6 * 3600,
    }
}

async fn seed_forward_proxy_binding_live_attempts(fixture: &ForwardProxyBindingFixture) {
    let manual_bucket_local = format_naive(
        Utc.timestamp_opt(fixture.manual_bucket_epoch + 300, 0)
            .single()
            .expect("manual bucket timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let direct_bucket_local = format_naive(
        Utc.timestamp_opt(fixture.manual_bucket_epoch + 3600 + 300, 0)
            .single()
            .expect("direct bucket timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    for (invoke_id, group_name, proxy_key, status) in [
        (
            "group-prod-manual-success",
            Some("prod"),
            Some(fixture.manual_key.as_str()),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "group-prod-manual-failure",
            Some("prod"),
            Some(fixture.manual_key.as_str()),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
        ),
        (
            "group-prod-manual-summary-final",
            Some("prod"),
            Some(fixture.manual_key.as_str()),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
        ),
        (
            "group-prod-direct-success",
            Some("prod"),
            Some(FORWARD_PROXY_DIRECT_KEY),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "group-other-manual-success",
            Some("staging"),
            Some(fixture.manual_key.as_str()),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
        (
            "group-prod-legacy-no-snapshot",
            Some("prod"),
            None,
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        ),
    ] {
        let bucket = if proxy_key == Some(FORWARD_PROXY_DIRECT_KEY) {
            &direct_bucket_local
        } else {
            &manual_bucket_local
        };
        seed_group_scoped_pool_attempt(
            &fixture.state.pool,
            invoke_id,
            bucket,
            group_name,
            proxy_key,
            status,
        )
        .await;
    }
}

async fn seed_forward_proxy_binding_archives(fixture: &ForwardProxyBindingFixture) {
    let historical_manual_key = format!("{}-legacy", fixture.manual_key);
    seed_forward_proxy_metadata_history(
        &fixture.state.pool,
        &historical_manual_key,
        &fixture.manual_display_name,
        "manual",
        "socks5://127.0.0.1:1080",
    )
    .await;
    let archived_bucket_local = format_naive(
        Utc.timestamp_opt(fixture.manual_bucket_epoch + 900, 0)
            .single()
            .expect("archived manual bucket timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    seed_group_scoped_pool_attempt_archive_batch(
        &fixture.state.pool,
        &fixture.state.config.archive_dir,
        "group-scoped-binding-prod",
        &[(
            "group-prod-archived-manual-success",
            archived_bucket_local.as_str(),
            Some("prod"),
            Some(historical_manual_key.as_str()),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        )],
    )
    .await;
    seed_legacy_group_scoped_pool_attempt_archive_batch_without_scope_columns(
        &fixture.state.pool,
        &fixture.state.config.archive_dir,
        "group-scoped-binding-prod-legacy",
        &[(
            "group-prod-legacy-archive-without-scope-columns",
            archived_bucket_local.as_str(),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        )],
    )
    .await;
}

async fn assert_forward_proxy_binding_group_scope(fixture: &ForwardProxyBindingFixture) {
    let baseline_nodes =
        build_forward_proxy_binding_nodes_response_with_options(fixture.state.as_ref(), &[], false)
            .await
            .expect("build baseline global binding nodes");
    let baseline_manual = baseline_nodes
        .iter()
        .find(|node| node.key == fixture.manual_key)
        .expect("baseline global manual node");
    let baseline_success = baseline_manual
        .last24h
        .iter()
        .map(|bucket| bucket.success_count)
        .sum::<i64>();
    let baseline_failure = baseline_manual
        .last24h
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum::<i64>();
    let Json(group_nodes) = list_forward_proxy_binding_nodes(
        State(fixture.state.clone()),
        "/api/pool/forward-proxy-binding-nodes?includeCurrent=true&groupName=prod"
            .parse()
            .expect("parse grouped binding uri"),
    )
    .await
    .expect("list grouped binding nodes");
    let Json(group_only_nodes) = list_forward_proxy_binding_nodes(
        State(fixture.state.clone()),
        "/api/pool/forward-proxy-binding-nodes?groupName=prod"
            .parse()
            .expect("parse grouped-only binding uri"),
    )
    .await
    .expect("list grouped binding nodes without explicit keys");
    assert!(!group_only_nodes.is_empty());
    let grouped_manual = group_nodes
        .iter()
        .find(|node| node.key == fixture.manual_key)
        .expect("grouped manual node");
    let grouped_only_manual = group_only_nodes
        .iter()
        .find(|node| node.key == fixture.manual_key)
        .expect("grouped-only manual node");
    assert_eq!(
        grouped_only_manual
            .last24h
            .iter()
            .map(|b| b.success_count)
            .sum::<i64>(),
        baseline_success
    );
    assert_eq!(
        grouped_only_manual
            .last24h
            .iter()
            .map(|b| b.failure_count)
            .sum::<i64>(),
        baseline_failure
    );
    assert_eq!(
        grouped_manual
            .last24h
            .iter()
            .map(|b| b.success_count)
            .sum::<i64>(),
        baseline_success
    );
    assert_eq!(
        grouped_manual
            .last24h
            .iter()
            .map(|b| b.failure_count)
            .sum::<i64>(),
        baseline_failure
    );
    assert_forward_proxy_direct_stats(&group_nodes, &baseline_nodes);
}

fn assert_forward_proxy_direct_stats(
    group_nodes: &[ForwardProxyBindingNodeResponse],
    baseline_nodes: &[ForwardProxyBindingNodeResponse],
) {
    let grouped_direct = group_nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("grouped direct node");
    let global_direct = baseline_nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("global direct node");
    assert_eq!(
        grouped_direct
            .last24h
            .iter()
            .map(|b| b.success_count)
            .sum::<i64>(),
        global_direct
            .last24h
            .iter()
            .map(|b| b.success_count)
            .sum::<i64>()
    );
    assert_eq!(
        grouped_direct
            .last24h
            .iter()
            .map(|b| b.failure_count)
            .sum::<i64>(),
        global_direct
            .last24h
            .iter()
            .map(|b| b.failure_count)
            .sum::<i64>()
    );
}

#[tokio::test]
pub(crate) async fn list_forward_proxy_binding_nodes_keeps_global_real_pool_attempts_when_group_name_is_present()
 {
    let fixture = create_forward_proxy_binding_fixture().await;
    seed_forward_proxy_binding_live_attempts(&fixture).await;
    seed_forward_proxy_binding_archives(&fixture).await;
    assert_forward_proxy_binding_group_scope(&fixture).await;
}

#[tokio::test]
pub(crate) async fn list_forward_proxy_binding_nodes_without_group_name_ignores_forward_proxy_health_checks()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    crate::ensure_schema(&state.pool)
        .await
        .expect("ensure full schema for forward proxy settings");

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        }),
    )
    .await
    .expect("persist forward proxy settings");

    let (manual_runtime_key, manual_binding_key) = {
        let manager = state.forward_proxy.lock().await;
        let binding_key = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("manual binding key");
        let runtime_key = manager
            .snapshot_runtime()
            .into_iter()
            .find(|runtime| runtime.proxy_key != FORWARD_PROXY_DIRECT_KEY)
            .map(|runtime| runtime.proxy_key)
            .expect("manual runtime key");
        (runtime_key, binding_key)
    };
    insert_forward_proxy_attempt(
        &state.pool,
        &manual_runtime_key,
        true,
        Some(12.5),
        None,
        false,
    )
    .await
    .expect("insert live forward proxy attempt");
    seed_pool_upstream_attempt_at(
        &state.pool,
        "binding-nodes-real-success",
        Utc::now() - ChronoDuration::minutes(2),
        Some(&manual_binding_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;

    let hourly_before: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(success_count), 0) FROM forward_proxy_attempt_hourly WHERE proxy_key = ?1",
        )
        .bind(&manual_runtime_key)
        .fetch_one(&state.pool)
        .await
        .expect("load hourly baseline");
    assert_eq!(
        hourly_before, 1,
        "test setup should seed the ungrouped view through the real write path"
    );

    let Json(nodes) = list_forward_proxy_binding_nodes(
        State(state.clone()),
        "/api/pool/forward-proxy-binding-nodes?includeCurrent=true"
            .parse()
            .expect("parse ungrouped binding uri"),
    )
    .await
    .expect("list forward proxy binding nodes");

    let manual = nodes
        .iter()
        .find(|node| node.key == manual_binding_key)
        .expect("manual node");
    assert_eq!(
        manual
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        1,
        "ungrouped binding nodes should only count real pool attempts, not forward-proxy health checks",
    );

    let hourly_after: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(success_count), 0) FROM forward_proxy_attempt_hourly WHERE proxy_key = ?1",
        )
        .bind(&manual_runtime_key)
        .fetch_one(&state.pool)
        .await
        .expect("load hourly after route catch-up");
    assert_eq!(hourly_after, 1);
}

#[test]
pub(crate) fn explicit_split_filters_override_legacy_status_mapping() {
    let enable_filters = collect_normalized_upstream_account_filters(
        &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED.to_string()],
        Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
        normalize_upstream_account_enable_status_filter,
    );
    assert_eq!(enable_filters, vec![UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED]);

    let health_filters = collect_normalized_upstream_account_filters(
        &[UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL.to_string()],
        Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
        normalize_upstream_account_health_status_filter,
    );
    assert_eq!(health_filters, vec![UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL]);
}

#[test]
pub(crate) fn matches_upstream_account_filters_uses_or_within_each_dimension() {
    let item = test_summary_with_statuses(
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED,
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
        UPSTREAM_ACCOUNT_SYNC_STATE_IDLE,
    );

    assert!(matches_upstream_account_filters(
        &item,
        &[
            UPSTREAM_ACCOUNT_WORK_STATUS_WORKING,
            UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
        ],
        &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED],
        &[
            UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
        ],
        Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
    ));

    assert!(!matches_upstream_account_filters(
        &item,
        &[UPSTREAM_ACCOUNT_WORK_STATUS_WORKING],
        &[UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED],
        &[UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL],
        Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
    ));
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_accepts_codex_export_json() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-1".to_string(),
        file_name: "2q5q6m3ow4a@duckmail.sbs.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "2q5q6m3ow4a@duckmail.sbs",
            "account_id": "acct_imported",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "2q5q6m3ow4a@duckmail.sbs",
                Some("acct_imported"),
                Some("user_imported"),
                Some("team"),
            ),
            "last_refresh": "2026-03-18T00:00:00Z"
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.source_id, "file-1");
    assert_eq!(normalized.file_name, "2q5q6m3ow4a@duckmail.sbs.json");
    assert_eq!(normalized.email, "2q5q6m3ow4a@duckmail.sbs");
    assert_eq!(normalized.chatgpt_account_id, "acct_imported");
    assert_eq!(normalized.display_name, "2q5q6m3ow4a@duckmail.sbs");
    assert_eq!(
        normalized.claims.chatgpt_user_id.as_deref(),
        Some("user_imported")
    );
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_accepts_non_codex_or_missing_type() {
    for (name, source_type) in [
        ("auth0", Some(json!("auth0"))),
        ("blank", Some(json!("  "))),
        ("missing", None),
    ] {
        let mut content = json!({
            "email": format!("{name}@duckmail.sbs"),
            "account_id": format!("acct_{name}"),
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                &format!("{name}@duckmail.sbs"),
                Some(&format!("acct_{name}")),
                Some(&format!("user_{name}")),
                Some("team"),
            ),
        });
        if let Some(source_type) = source_type {
            content["type"] = source_type;
        }
        let item = ImportOauthCredentialFileRequest {
            source_id: format!("file-{name}"),
            file_name: format!("{name}.json"),
            content: content.to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials with any type");

        assert_eq!(normalized.email, format!("{name}@duckmail.sbs"));
        assert_eq!(normalized.chatgpt_account_id, format!("acct_{name}"));
    }
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_accepts_sub2api_oauth_account_objects() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "sub2api-oauth".to_string(),
        file_name: "sub2api-oauth.json".to_string(),
        content: json!({
            "platform": "openai",
            "type": "oauth",
            "credentials": {
                "email": "student@example.com",
                "chatgpt_account_id": "acct_shared_k12",
                "chatgpt_user_id": "user_student",
                "plan_type": "k12",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "id_token": test_id_token(
                    "student@example.com",
                    Some("acct_shared_k12"),
                    Some("user_student"),
                    Some("k12"),
                ),
                "expires_at": "2026-03-20T00:00:00Z"
            }
        })
        .to_string(),
    };

    let normalized = normalize_imported_oauth_credentials(&item)
        .expect("normalize imported sub2api oauth account");

    assert_eq!(normalized.email, "student@example.com");
    assert_eq!(normalized.chatgpt_account_id, "acct_shared_k12");
    assert_eq!(normalized.chatgpt_user_id.as_deref(), Some("user_student"));
    assert_eq!(normalized.claims.chatgpt_plan_type.as_deref(), Some("k12"));
}

#[test]
pub(crate) fn imported_match_key_prefers_chatgpt_user_id_before_email_or_account_id() {
    assert_eq!(
        imported_match_key(Some("user_member"), "member@example.com", "acct_shared"),
        "user:user_member"
    );
    assert_eq!(
        imported_match_key(None, "member@example.com", "acct_shared"),
        "account:acct_shared"
    );
    assert_eq!(
        imported_match_key(None, "", "acct_shared"),
        "account:acct_shared"
    );
}

#[test]
pub(crate) fn normalize_imported_oauth_credentials_accepts_missing_or_blank_refresh_token() {
    for (name, refresh_token) in [
        ("missing", None),
        ("null", Some(serde_json::Value::Null)),
        ("blank", Some(json!("  "))),
    ] {
        let mut content = json!({
            "type": "codex",
            "email": format!("{name}@duckmail.sbs"),
            "account_id": format!("acct_{name}"),
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "id_token": test_id_token(
                &format!("{name}@duckmail.sbs"),
                Some(&format!("acct_{name}")),
                Some(&format!("user_{name}")),
                Some("team"),
            ),
        });
        if let Some(refresh_token) = refresh_token {
            content["refresh_token"] = refresh_token;
        }
        let item = ImportOauthCredentialFileRequest {
            source_id: format!("file-{name}"),
            file_name: format!("{name}.json"),
            content: content.to_string(),
        };

        let normalized = normalize_imported_oauth_credentials(&item)
            .expect("normalize imported oauth credentials without refresh token");

        assert_eq!(normalized.credentials.refresh_token, None);
        assert!(!oauth_credentials_have_refresh_token(
            &normalized.credentials
        ));
    }
}

#[tokio::test]
pub(crate) async fn probe_imported_oauth_credentials_skips_refresh_without_refresh_token() {
    #[derive(Clone)]
    struct ProbeServerState {
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
    }

    async fn usage_handler(State(state): State<ProbeServerState>) -> (StatusCode, String) {
        state.usage_requests.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 8,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            })
            .to_string(),
        )
    }

    async fn token_handler(State(state): State<ProbeServerState>) -> (StatusCode, String) {
        state.token_requests.fetch_add(1, Ordering::SeqCst);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "refresh endpoint should not be called" }).to_string(),
        )
    }

    let usage_requests = Arc::new(AtomicUsize::new(0));
    let token_requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(usage_handler))
        .route("/oauth/token", post(token_handler))
        .with_state(ProbeServerState {
            usage_requests: usage_requests.clone(),
            token_requests: token_requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported probe server");
    let addr = listener.local_addr().expect("imported probe server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported probe server");
    });
    let origin = format!("http://{addr}");
    let state =
        test_app_state_with_usage_and_oauth_base(&format!("{origin}/backend-api"), &origin).await;
    let normalized = normalize_imported_oauth_credentials(&ImportOauthCredentialFileRequest {
        source_id: "source-no-rt".to_string(),
        file_name: "no-rt.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "no-rt@duckmail.sbs",
            "account_id": "acct_no_rt",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-no-rt",
            "id_token": test_id_token(
                "no-rt@duckmail.sbs",
                Some("acct_no_rt"),
                Some("user_no_rt"),
                Some("team"),
            ),
        })
        .to_string(),
    })
    .expect("normalize no refresh token import");
    let scope = ForwardProxyRouteScope::Automatic;

    let outcome = probe_imported_oauth_credentials(&state, &normalized, &scope, &scope)
        .await
        .expect("probe no refresh token import");

    assert!(!oauth_credentials_have_refresh_token(&outcome.credentials));
    assert_eq!(token_requests.load(Ordering::SeqCst), 0);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);

    server.abort();
}
