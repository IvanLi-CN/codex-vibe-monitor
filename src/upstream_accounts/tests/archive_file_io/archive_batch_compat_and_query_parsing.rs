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

async fn wait_for_imported_oauth_validation_job_terminal(
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
        effective_routing_rule: EffectiveRoutingRule {
            allow_cut_out: true,
            allow_cut_in: true,
            priority_tier: TagPriorityTier::Normal,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            request_compression_algorithm: RequestCompressionAlgorithm::Identity,
            concurrency_limit: 0,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            available_models: vec![],
            available_models_defined: false,
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
                request_compression_algorithm: "root".to_string(),
                concurrency_limit: "root".to_string(),
                upstream_429_retry: "root".to_string(),
                available_models: "root".to_string(),
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
        },
        response_endpoint_capability: UpstreamCapabilityState {
            observed: CapabilitySupport::Unknown,
            override_value: None,
            effective: CapabilitySupport::Unknown,
            observed_at: None,
            reason: None,
        },
        image_endpoint_capability: UpstreamCapabilityState {
            observed: CapabilitySupport::Unknown,
            override_value: None,
            effective: CapabilitySupport::Unknown,
            observed_at: None,
            reason: None,
        },
        response_image_tool_capability: UpstreamCapabilityState {
            observed: CapabilitySupport::Unknown,
            override_value: None,
            effective: CapabilitySupport::Unknown,
            observed_at: None,
            reason: None,
        },
    }
}

async fn seed_group_scoped_pool_attempt(
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

async fn seed_pool_upstream_attempt_at(
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

async fn seed_forward_proxy_metadata_history(
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

async fn seed_group_scoped_pool_attempt_archive_batch(
    pool: &SqlitePool,
    archive_dir: &Path,
    batch_name: &str,
    rows: &[(&str, &str, Option<&str>, Option<&str>, &str)],
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

    for (index, row) in rows.iter().enumerate() {
        let phase = if row.4 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
        } else {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
        };
        sqlx::query(
            r#"
                INSERT INTO pool_upstream_request_attempts (
                    id,
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
                    ?1, ?2, ?3, '/v1/responses', ?4, 'archived-group-scope', ?5, ?6, 41,
                    'archived-route-group-scope', 1, 1, 0, '203.0.113.19', ?3, ?3, ?7, ?8, ?9, ?10,
                    datetime('now')
                )
                "#,
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
        .bind(
            (row.4 != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
                .then_some("group-scoped archived failure"),
        )
        .execute(&archive_pool)
        .await
        .expect("insert archive group-scoped pool attempt row");
    }

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

async fn seed_legacy_group_scoped_pool_attempt_archive_batch_without_scope_columns(
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

    for (index, row) in rows.iter().enumerate() {
        let phase = if row.2 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_COMPLETED
        } else {
            POOL_UPSTREAM_REQUEST_ATTEMPT_PHASE_FAILED
        };
        sqlx::query(
            r#"
                INSERT INTO pool_upstream_request_attempts (
                    id,
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
                    phase,
                    http_status,
                    error_message,
                    created_at
                )
                VALUES (
                    ?1, ?2, ?3, '/v1/responses', ?4, 'legacy-archived-group-scope', 41,
                    'legacy-archived-route-group-scope', 1, 1, 0, '203.0.113.29', ?3, ?3, ?5, ?6,
                    ?7, ?8, datetime('now')
                )
                "#,
        )
        .bind(20_000_i64 + index as i64)
        .bind(row.0)
        .bind(row.1)
        .bind(INVOCATION_ROUTE_MODE_POOL)
        .bind(row.2)
        .bind(phase)
        .bind((row.2 == POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS).then_some(200_i64))
        .bind(
            (row.2 != POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS)
                .then_some("legacy archived failure"),
        )
        .execute(&archive_pool)
        .await
        .expect("insert legacy archive pool attempt row");
    }

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

#[test]
fn derive_secret_key_is_stable() {
    let lhs = derive_secret_key("alpha");
    let rhs = derive_secret_key("alpha");
    assert_eq!(lhs, rhs);
}

#[test]
fn credential_round_trip_works() {
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
fn deserialize_optional_field_distinguishes_missing_null_and_value() {
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
fn list_query_deserializes_repeated_status_filters() {
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
fn list_query_keeps_single_status_filter_compatible() {
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
fn list_query_parses_include_all_flag() {
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
fn list_query_parses_single_tag_filter() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?tagIds=5"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize single tag filter");

    assert_eq!(query.tag_ids, vec![5]);
}

#[test]
fn list_query_parses_repeated_tag_filters() {
    let query = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?tagIds=1&tagIds=2"
            .parse()
            .expect("parse uri"),
    )
    .expect("deserialize repeated tag filters");

    assert_eq!(query.tag_ids, vec![1, 2]);
}

#[test]
fn list_query_rejects_invalid_tag_filter() {
    let err = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?tagIds=abc"
            .parse()
            .expect("parse uri"),
    )
    .expect_err("invalid tagIds should fail");

    assert!(err.contains("invalid tagIds value `abc`; expected integer"));
}

#[test]
fn list_query_parses_exact_group_filter() {
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
fn list_query_parses_repeated_exact_group_filters() {
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
fn list_query_rejects_invalid_include_all_flag() {
    let err = parse_list_upstream_accounts_query(
        &"/api/pool/upstream-accounts?includeAll=maybe"
            .parse()
            .expect("parse uri"),
    )
    .expect_err("invalid includeAll should fail");

    assert!(err.contains("invalid includeAll value"));
}

#[test]
fn forward_proxy_binding_nodes_query_keeps_repeated_keys_and_include_current() {
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
fn forward_proxy_binding_nodes_query_ignores_blank_group_name() {
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
async fn list_forward_proxy_binding_nodes_returns_current_nodes_and_deduplicated_requested_missing_keys()
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

#[tokio::test]
async fn list_forward_proxy_binding_nodes_keeps_global_real_pool_attempts_when_group_name_is_present()
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
    let now_epoch = Utc::now().timestamp();
    let range_end_epoch = align_bucket_epoch(now_epoch, 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    let manual_bucket_epoch = range_start_epoch + 6 * 3600;
    let direct_bucket_epoch = range_start_epoch + 7 * 3600;

    let manual_bucket_local = format_naive(
        Utc.timestamp_opt(manual_bucket_epoch + 300, 0)
            .single()
            .expect("manual bucket timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let direct_bucket_local = format_naive(
        Utc.timestamp_opt(direct_bucket_epoch + 300, 0)
            .single()
            .expect("direct bucket timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    seed_group_scoped_pool_attempt(
        &state.pool,
        "group-prod-manual-success",
        &manual_bucket_local,
        Some("prod"),
        Some(&manual_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_group_scoped_pool_attempt(
        &state.pool,
        "group-prod-manual-failure",
        &manual_bucket_local,
        Some("prod"),
        Some(&manual_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE,
    )
    .await;
    seed_group_scoped_pool_attempt(
        &state.pool,
        "group-prod-manual-summary-final",
        &manual_bucket_local,
        Some("prod"),
        Some(&manual_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_BUDGET_EXHAUSTED_FINAL,
    )
    .await;
    seed_group_scoped_pool_attempt(
        &state.pool,
        "group-prod-direct-success",
        &direct_bucket_local,
        Some("prod"),
        Some(FORWARD_PROXY_DIRECT_KEY),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_group_scoped_pool_attempt(
        &state.pool,
        "group-other-manual-success",
        &manual_bucket_local,
        Some("staging"),
        Some(&manual_key),
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    seed_group_scoped_pool_attempt(
        &state.pool,
        "group-prod-legacy-no-snapshot",
        &manual_bucket_local,
        Some("prod"),
        None,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
    )
    .await;
    let historical_manual_key = format!("{manual_key}-legacy");
    seed_forward_proxy_metadata_history(
        &state.pool,
        &historical_manual_key,
        &manual_display_name,
        "manual",
        "socks5://127.0.0.1:1080",
    )
    .await;
    let archived_manual_bucket_local = format_naive(
        Utc.timestamp_opt(manual_bucket_epoch + 900, 0)
            .single()
            .expect("archived manual bucket timestamp")
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let _archive_path = seed_group_scoped_pool_attempt_archive_batch(
        &state.pool,
        &state.config.archive_dir,
        "group-scoped-binding-prod",
        &[(
            "group-prod-archived-manual-success",
            archived_manual_bucket_local.as_str(),
            Some("prod"),
            Some(historical_manual_key.as_str()),
            POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        )],
    )
    .await;
    let _legacy_archive_path =
        seed_legacy_group_scoped_pool_attempt_archive_batch_without_scope_columns(
            &state.pool,
            &state.config.archive_dir,
            "group-scoped-binding-prod-legacy",
            &[(
                "group-prod-legacy-archive-without-scope-columns",
                archived_manual_bucket_local.as_str(),
                POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
            )],
        )
        .await;
    let global_baseline_nodes =
        build_forward_proxy_binding_nodes_response_with_options(state.as_ref(), &[], false)
            .await
            .expect("build baseline global binding nodes");
    let global_baseline_manual = global_baseline_nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("baseline global manual node");
    let global_baseline_manual_success = global_baseline_manual
        .last24h
        .iter()
        .map(|bucket| bucket.success_count)
        .sum::<i64>();
    let global_baseline_manual_failure = global_baseline_manual
        .last24h
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum::<i64>();

    let Json(group_nodes) = list_forward_proxy_binding_nodes(
        State(state.clone()),
        "/api/pool/forward-proxy-binding-nodes?includeCurrent=true&groupName=prod"
            .parse()
            .expect("parse grouped binding uri"),
    )
    .await
    .expect("list grouped binding nodes");

    let Json(group_only_nodes) = list_forward_proxy_binding_nodes(
        State(state.clone()),
        "/api/pool/forward-proxy-binding-nodes?groupName=prod"
            .parse()
            .expect("parse grouped-only binding uri"),
    )
    .await
    .expect("list grouped binding nodes without explicit keys");
    assert!(
        !group_only_nodes.is_empty(),
        "groupName alone should still return the grouped binding catalog",
    );

    let grouped_manual = group_nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("grouped manual node");
    let grouped_only_manual = group_only_nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("grouped-only manual node");
    assert_eq!(
        grouped_only_manual
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        global_baseline_manual_success,
        "groupName-only requests should preserve global real pool manual successes",
    );
    assert_eq!(
        grouped_only_manual
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum::<i64>(),
        global_baseline_manual_failure,
        "groupName-only requests should preserve global real pool manual failures",
    );
    assert_eq!(
        grouped_manual
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        global_baseline_manual_success,
        "groupName requests should preserve global real pool manual successes after alias remapping",
    );
    assert_eq!(
        grouped_manual
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum::<i64>(),
        global_baseline_manual_failure,
        "groupName requests should preserve global real pool manual failures",
    );
    let grouped_direct = group_nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("grouped direct node");
    let global_direct = global_baseline_nodes
        .iter()
        .find(|node| node.key == FORWARD_PROXY_DIRECT_KEY)
        .expect("global direct node");
    assert_eq!(
        grouped_direct
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        global_direct
            .last24h
            .iter()
            .map(|bucket| bucket.success_count)
            .sum::<i64>(),
        "direct real traffic should remain consistent when groupName is present",
    );
    assert_eq!(
        grouped_direct
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum::<i64>(),
        global_direct
            .last24h
            .iter()
            .map(|bucket| bucket.failure_count)
            .sum::<i64>(),
    );
}

#[tokio::test]
async fn list_forward_proxy_binding_nodes_without_group_name_ignores_forward_proxy_health_checks() {
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

#[tokio::test]
async fn live_first_proxy_binding_key_snapshot_canonicalizes_endpoint_storage_key() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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

    let (selected_proxy, binding_key) = {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        let endpoint = manager
            .endpoints
            .iter()
            .find(|endpoint| endpoint.key != FORWARD_PROXY_DIRECT_KEY)
            .cloned()
            .expect("manual forward proxy endpoint");
        let binding_key = manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("manual binding key");
        let selected_proxy = SelectedForwardProxy::from_endpoint(&endpoint);
        assert_ne!(
            selected_proxy.key, binding_key,
            "regression test requires the runtime endpoint storage key to differ from the canonical binding key"
        );
        (selected_proxy, binding_key)
    };

    let snapshot =
        live_first_proxy_binding_key_snapshot(state.as_ref(), Some(&selected_proxy)).await;
    assert_eq!(snapshot.as_deref(), Some(binding_key.as_str()));
}

#[test]
fn explicit_split_filters_override_legacy_status_mapping() {
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
fn matches_upstream_account_filters_uses_or_within_each_dimension() {
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
fn normalize_imported_oauth_credentials_accepts_codex_export_json() {
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
fn normalize_imported_oauth_credentials_accepts_non_codex_or_missing_type() {
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
fn normalize_imported_oauth_credentials_accepts_sub2api_oauth_account_objects() {
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
fn imported_match_key_prefers_chatgpt_user_id_before_email_or_account_id() {
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
fn normalize_imported_oauth_credentials_accepts_missing_or_blank_refresh_token() {
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
async fn probe_imported_oauth_credentials_skips_refresh_without_refresh_token() {
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

#[test]
fn normalize_imported_oauth_credentials_ignores_non_string_unused_fields() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-non-string-unused".to_string(),
        file_name: "non-string-unused.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "non-string-unused@duckmail.sbs",
            "account_id": "acct_non_string_unused",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "non-string-unused@duckmail.sbs",
                Some("acct_non_string_unused"),
                Some("user_non_string_unused"),
                Some("team"),
            ),
            "last_refresh": {
                "at": "2026-03-18T00:00:00Z"
            },
            "token_type": 42
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.credentials.token_type.as_deref(), Some("Bearer"));
    assert_eq!(normalized.chatgpt_account_id, "acct_non_string_unused");
}

#[test]
fn normalize_imported_oauth_credentials_uses_access_token_exp_when_expired_blank() {
    let access_exp = 1_777_777_777;
    let id_exp = 1_666_666_666;
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-blank-expired".to_string(),
        file_name: "blank-expired.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "blank-expired@duckmail.sbs",
            "account_id": "acct_blank_expired",
            "expired": "",
            "access_token": test_jwt_token(json!({ "exp": access_exp })),
            "refresh_token": "refresh-token",
            "id_token": test_jwt_token(json!({
                "exp": id_exp,
                "email": "blank-expired@duckmail.sbs",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct_blank_expired",
                    "chatgpt_user_id": "user_blank_expired",
                    "chatgpt_plan_type": "team"
                }
            }))
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.token_expires_at, "2026-05-03T03:09:37Z");
}

#[test]
fn normalize_imported_oauth_credentials_uses_id_token_exp_when_expired_missing() {
    let id_exp = 1_666_666_666;
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-missing-expired".to_string(),
        file_name: "missing-expired.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "missing-expired@duckmail.sbs",
            "account_id": "acct_missing_expired",
            "access_token": "opaque-access-token",
            "refresh_token": "refresh-token",
            "id_token": test_jwt_token(json!({
                "exp": id_exp,
                "email": "missing-expired@duckmail.sbs",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct_missing_expired",
                    "chatgpt_user_id": "user_missing_expired",
                    "chatgpt_plan_type": "team"
                }
            }))
        })
        .to_string(),
    };

    let normalized =
        normalize_imported_oauth_credentials(&item).expect("normalize imported oauth credentials");
    assert_eq!(normalized.token_expires_at, "2022-10-25T02:57:46Z");
}

#[test]
fn normalize_imported_oauth_credentials_rejects_non_empty_invalid_expired() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-invalid-expired".to_string(),
        file_name: "invalid-expired.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "invalid-expired@duckmail.sbs",
            "account_id": "acct_invalid_expired",
            "expired": "not-a-date",
            "access_token": test_jwt_token(json!({ "exp": 1_777_777_777 })),
            "refresh_token": "refresh-token",
            "id_token": test_jwt_token(json!({
                "exp": 1_666_666_666,
                "email": "invalid-expired@duckmail.sbs",
                "https://api.openai.com/auth": {
                    "chatgpt_account_id": "acct_invalid_expired",
                    "chatgpt_user_id": "user_invalid_expired",
                    "chatgpt_plan_type": "team"
                }
            }))
        })
        .to_string(),
    };

    let error = normalize_imported_oauth_credentials(&item)
        .expect_err("expected invalid expired timestamp");
    assert_eq!(error, "expired must be a valid RFC3339 timestamp");
}

#[test]
fn normalize_imported_oauth_credentials_rejects_missing_expired_without_token_exp() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-missing-expired-no-exp".to_string(),
        file_name: "missing-expired-no-exp.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "missing-expired-no-exp@duckmail.sbs",
            "account_id": "acct_missing_expired_no_exp",
            "access_token": "opaque-access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "missing-expired-no-exp@duckmail.sbs",
                Some("acct_missing_expired_no_exp"),
                Some("user_missing_expired_no_exp"),
                Some("team"),
            )
        })
        .to_string(),
    };

    let error = normalize_imported_oauth_credentials(&item)
        .expect_err("expected missing expiry to be rejected");
    assert_eq!(error, "expired is required when token exp is unavailable");
}

#[test]
fn normalize_imported_oauth_credentials_rejects_id_token_mismatch() {
    let item = ImportOauthCredentialFileRequest {
        source_id: "file-2".to_string(),
        file_name: "mismatch.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "mismatch@duckmail.sbs",
            "account_id": "acct_imported",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "different@duckmail.sbs",
                Some("acct_imported"),
                Some("user_imported"),
                Some("team"),
            )
        })
        .to_string(),
    };

    let error =
        normalize_imported_oauth_credentials(&item).expect_err("expected imported oauth mismatch");
    assert_eq!(error, "email does not match id_token");
}

#[tokio::test]
async fn imported_oauth_validation_job_caches_successful_probe_for_import_reuse() {
    let binding = ResolvedRequiredGroupProxyBinding {
        group_name: "import-group".to_string(),
        bound_proxy_keys: test_required_group_bound_proxy_keys(),
        node_shunt_enabled: false,
    };
    let job = Arc::new(ImportedOauthValidationJob::new(
        ImportedOauthValidationResponse {
            input_files: 1,
            unique_in_input: 1,
            duplicate_in_input: 0,
            rows: vec![ImportedOauthValidationRow {
                source_id: "source-1".to_string(),
                file_name: "alpha.json".to_string(),
                email: None,
                chatgpt_account_id: None,
                chatgpt_user_id: None,
                display_name: None,
                token_expires_at: None,
                matched_account: None,
                status: "pending".to_string(),
                detail: None,
                attempts: 0,
            }],
        },
        &binding,
    ));
    let normalized = NormalizedImportedOauthCredentials {
        source_id: "source-1".to_string(),
        file_name: "alpha.json".to_string(),
        email: "alpha@duckmail.sbs".to_string(),
        display_name: "alpha@duckmail.sbs".to_string(),
        chatgpt_account_id: "acct_alpha".to_string(),
        chatgpt_user_id: Some("user_alpha".to_string()),
        token_expires_at: "2026-03-20T00:00:00Z".to_string(),
        credentials: StoredOauthCredentials {
            access_token: "access-token".to_string(),
            refresh_token: Some("refresh-token".to_string()),
            id_token: test_id_token(
                "alpha@duckmail.sbs",
                Some("acct_alpha"),
                Some("user_alpha"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        claims: test_claims("alpha@duckmail.sbs", Some("acct_alpha"), Some("user_alpha")),
    };
    let probe = ImportedOauthProbeOutcome {
        token_expires_at: "2026-03-20T00:00:00Z".to_string(),
        credentials: normalized.credentials.clone(),
        claims: normalized.claims.clone(),
        usage_snapshot: None,
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: Some("usage snapshot unavailable during validation".to_string()),
    };

    update_imported_oauth_validation_job_row(
        &job,
        0,
        ImportedOauthValidationRow {
            source_id: "source-1".to_string(),
            file_name: "alpha.json".to_string(),
            email: Some("alpha@duckmail.sbs".to_string()),
            chatgpt_account_id: Some("acct_alpha".to_string()),
            chatgpt_user_id: Some("user_alpha".to_string()),
            display_name: Some("alpha@duckmail.sbs".to_string()),
            token_expires_at: Some("2026-03-20T00:00:00Z".to_string()),
            matched_account: None,
            status: IMPORT_VALIDATION_STATUS_OK.to_string(),
            detail: probe.usage_snapshot_warning.clone(),
            attempts: 1,
        },
        Some(ImportedOauthValidatedImportData { normalized, probe }),
    )
    .await;

    let cached = job
        .validated_imports
        .lock()
        .await
        .get("source-1")
        .cloned()
        .expect("cached validated import");
    assert_eq!(cached.normalized.email, "alpha@duckmail.sbs");
    assert_eq!(cached.normalized.chatgpt_account_id, "acct_alpha");
    assert_eq!(
        cached.probe.credentials.refresh_token.as_deref(),
        Some("refresh-token")
    );
}

#[tokio::test]
async fn imported_oauth_validation_job_only_consumes_node_shunt_slots_after_success() {
    #[derive(Clone)]
    struct ImportedOauthValidationServerState {
        usage_requests: Arc<AtomicUsize>,
        token_requests: Arc<AtomicUsize>,
    }

    async fn usage_handler(
        State(state): State<ImportedOauthValidationServerState>,
    ) -> (StatusCode, String) {
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

    async fn token_handler(
        State(state): State<ImportedOauthValidationServerState>,
        axum::extract::Form(form): axum::extract::Form<std::collections::HashMap<String, String>>,
    ) -> (StatusCode, String) {
        state.token_requests.fetch_add(1, Ordering::SeqCst);
        let refresh_token = form.get("refresh_token").cloned().unwrap_or_default();
        if refresh_token == "bad-refresh" {
            return (
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "invalid_grant",
                    "error_description": "refresh token rejected"
                })
                .to_string(),
            );
        }

        (
            StatusCode::OK,
            json!({
                "access_token": "refreshed-access",
                "refresh_token": "refreshed-refresh",
                "id_token": test_id_token(
                    "fallback@duckmail.sbs",
                    Some("acct_fallback"),
                    Some("user_fallback"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600,
            })
            .to_string(),
        )
    }

    fn imported_item(
        source_id: &str,
        file_name: &str,
        email: &str,
        account_id: &str,
        expires_at: &str,
        refresh_token: &str,
    ) -> ImportOauthCredentialFileRequest {
        ImportOauthCredentialFileRequest {
            source_id: source_id.to_string(),
            file_name: file_name.to_string(),
            content: json!({
                "type": "codex",
                "email": email,
                "account_id": account_id,
                "expired": expires_at,
                "access_token": format!("access-{source_id}"),
                "refresh_token": refresh_token,
                "id_token": test_id_token(
                    email,
                    Some(account_id),
                    Some(format!("user_{source_id}").as_str()),
                    Some("team"),
                ),
            })
            .to_string(),
        }
    }

    let usage_requests = Arc::new(AtomicUsize::new(0));
    let token_requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(usage_handler))
        .route("/oauth/token", post(token_handler))
        .with_state(ImportedOauthValidationServerState {
            usage_requests: usage_requests.clone(),
            token_requests: token_requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported validation server");
    let addr = listener
        .local_addr()
        .expect("imported validation server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported validation server");
    });
    let origin = format!("http://{addr}");

    let state =
        test_app_state_with_usage_and_oauth_base(&format!("{origin}/backend-api"), &origin).await;
    let Json(response) = create_imported_oauth_validation_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(ValidateImportedOauthAccountsRequest {
            group_name: Some("import-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            items: vec![
                imported_item(
                    "source-bad",
                    "bad.json",
                    "bad@duckmail.sbs",
                    "acct_bad",
                    "2026-03-20T00:00:00Z",
                    "bad-refresh",
                ),
                imported_item(
                    "source-good",
                    "good.json",
                    "good@duckmail.sbs",
                    "acct_good",
                    "2099-04-20T00:00:00Z",
                    "good-refresh",
                ),
            ],
        }),
    )
    .await
    .expect("start imported oauth validation job");
    let job = state
        .upstream_accounts
        .get_validation_job(&response.job_id)
        .await
        .expect("validation job should exist");
    let _terminal = wait_for_imported_oauth_validation_job_terminal(&job).await;

    let rows = job.snapshot.lock().await.rows.clone();
    let bad_row = rows
        .iter()
        .find(|row| row.source_id == "source-bad")
        .expect("bad row");
    let good_row = rows
        .iter()
        .find(|row| row.source_id == "source-good")
        .expect("good row");

    assert_eq!(bad_row.status, IMPORT_VALIDATION_STATUS_INVALID);
    assert!(
        bad_row
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("refresh token rejected")
    );
    assert_eq!(good_row.status, IMPORT_VALIDATION_STATUS_OK);
    assert_ne!(
        good_row.detail.as_deref(),
        Some(group_node_shunt_unassigned_error_message())
    );
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);

    server.abort();
}

#[tokio::test]
async fn imported_oauth_validation_job_probes_node_shunt_group_when_no_slot_is_available() {
    async fn usage_handler() -> (StatusCode, String) {
        (
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 12,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            })
            .to_string(),
        )
    }

    let app = Router::new().route("/backend-api/wham/usage", get(usage_handler));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported validation server");
    let addr = listener
        .local_addr()
        .expect("imported validation server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported validation server");
    });
    let origin = format!("http://{addr}");

    let state = test_app_state_with_usage_base(&format!("{origin}/backend-api")).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Account",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    set_test_account_group_name(&state.pool, occupying_account_id, Some("import-group")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "import-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt metadata");
    drop(conn);

    let Json(response) = create_imported_oauth_validation_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(ValidateImportedOauthAccountsRequest {
            group_name: Some("import-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            items: vec![ImportOauthCredentialFileRequest {
                source_id: "source-new".to_string(),
                file_name: "new-session.json".to_string(),
                content: json!({
                    "type": "codex",
                    "email": "new-session@example.com",
                    "account_id": "acct_new_session",
                    "expired": "2099-04-20T00:00:00Z",
                    "access_token": "access-new-session",
                    "id_token": test_id_token(
                        "new-session@example.com",
                        Some("acct_new_session"),
                        Some("user_new_session"),
                        Some("team"),
                    ),
                })
                .to_string(),
            }],
        }),
    )
    .await
    .expect("start imported oauth validation job");
    let job = state
        .upstream_accounts
        .get_validation_job(&response.job_id)
        .await
        .expect("validation job should exist");
    let _terminal = wait_for_imported_oauth_validation_job_terminal(&job).await;

    let rows = job.snapshot.lock().await.rows.clone();
    let row = rows.first().expect("validation row");
    assert_eq!(row.status, IMPORT_VALIDATION_STATUS_OK);
    assert_ne!(
        row.detail.as_deref(),
        Some(group_node_shunt_unassigned_error_message())
    );

    server.abort();
}

#[tokio::test]
async fn import_validated_oauth_accounts_persists_node_shunt_group_when_no_slot_is_available() {
    async fn usage_handler() -> (StatusCode, String) {
        (
            StatusCode::OK,
            json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 12,
                        "windowDurationMins": 300,
                        "resetsAt": 1771322400
                    }
                }
            })
            .to_string(),
        )
    }

    let app = Router::new().route("/backend-api/wham/usage", get(usage_handler));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind imported validation server");
    let addr = listener
        .local_addr()
        .expect("imported validation server addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve imported validation server");
    });
    let origin = format!("http://{addr}");

    let state = test_app_state_with_usage_base(&format!("{origin}/backend-api")).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Account",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    set_test_account_group_name(&state.pool, occupying_account_id, Some("import-group")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "import-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt metadata");
    drop(conn);

    let imported = ImportOauthCredentialFileRequest {
        source_id: "source-new".to_string(),
        file_name: "new-session.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "new-session@example.com",
            "account_id": "acct_new_session",
            "expired": "2099-04-20T00:00:00Z",
            "access_token": "access-new-session",
            "id_token": test_id_token(
                "new-session@example.com",
                Some("acct_new_session"),
                Some("user_new_session"),
                Some("team"),
            ),
        })
        .to_string(),
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::HOST,
        axum::http::HeaderValue::from_static("127.0.0.1:8080"),
    );

    let Json(response) = import_validated_oauth_accounts(
        State(state.clone()),
        headers,
        Json(ImportValidatedOauthAccountsRequest {
            items: vec![imported],
            selected_source_ids: vec!["source-new".to_string()],
            validation_job_id: None,
            group_name: Some("import-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            group_note: None,
            concurrency_limit: None,
            tag_ids: vec![],
        }),
    )
    .await
    .expect("import should persist even when node shunt slots are full");

    assert_eq!(response.summary.created, 1);
    assert_eq!(response.summary.failed, 0);
    let account_id = response
        .results
        .first()
        .and_then(|result| result.account_id)
        .expect("created account id");
    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
        .await
        .expect("build node shunt assignments");
    assert!(
        !assignments.account_proxy_keys.contains_key(&account_id),
        "new account should persist without stealing an occupied node shunt slot",
    );
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load imported account")
        .expect("imported account");
    let metadata = load_group_metadata(&state.pool, Some("import-group"))
        .await
        .expect("load group metadata");
    let err = resolve_account_forward_proxy_scope_from_assignments(
        row.id,
        row.group_name.as_deref(),
        &metadata,
        &assignments,
    )
    .expect_err("persisted account should remain unroutable until a node shunt slot opens");
    assert!(is_group_node_shunt_unassigned_message(&err.to_string()));

    server.abort();
}

#[tokio::test]
async fn imported_oauth_validation_job_keeps_node_shunt_group_blocked_without_selectable_nodes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let Json(response) = create_imported_oauth_validation_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(ValidateImportedOauthAccountsRequest {
            group_name: Some("stale-import-group".to_string()),
            group_bound_proxy_keys: Some(vec!["stale-node".to_string()]),
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            items: vec![ImportOauthCredentialFileRequest {
                source_id: "source-stale".to_string(),
                file_name: "stale-session.json".to_string(),
                content: json!({
                    "type": "codex",
                    "email": "stale-session@example.com",
                    "account_id": "acct_stale_session",
                    "expired": "2099-04-20T00:00:00Z",
                    "access_token": "access-stale-session",
                    "id_token": test_id_token(
                        "stale-session@example.com",
                        Some("acct_stale_session"),
                        Some("user_stale_session"),
                        Some("team"),
                    ),
                })
                .to_string(),
            }],
        }),
    )
    .await
    .expect("start imported oauth validation job");
    let job = state
        .upstream_accounts
        .get_validation_job(&response.job_id)
        .await
        .expect("validation job should exist");
    let _terminal = wait_for_imported_oauth_validation_job_terminal(&job).await;

    let rows = job.snapshot.lock().await.rows.clone();
    let row = rows.first().expect("validation row");
    assert_eq!(row.status, IMPORT_VALIDATION_STATUS_ERROR);
    assert_eq!(
        row.detail.as_deref(),
        Some(group_node_shunt_unassigned_error_message())
    );
}

#[tokio::test]
async fn update_upstream_account_group_allows_note_only_edits_when_node_shunt_group_has_no_selectable_nodes()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Node Shunt Guard").await;
    let group_name = "empty-node-shunt";
    let stale_proxy_key = "stale-node".to_string();
    set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        group_name,
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![stale_proxy_key.clone()],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let Json(response) = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath(group_name.to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: Some("still editable".to_string()),
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: None,
        }),
    )
    .await
    .expect("note-only edit should succeed even without selectable nodes");

    assert_eq!(response.group_name, group_name);
    assert_eq!(response.note.as_deref(), Some("still editable"));
    assert_eq!(response.bound_proxy_keys, vec![stale_proxy_key]);
    assert!(response.node_shunt_enabled);
}

#[tokio::test]
async fn update_upstream_account_group_rejects_clearing_bindings_while_node_shunt_stays_enabled() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Node Shunt Bound").await;
    set_test_account_group_name(&state.pool, account_id, Some("bound-node-shunt")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "bound-node-shunt",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath("bound-node-shunt".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: Some(vec![]),
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: None,
        }),
    )
    .await
    .expect_err("node shunt group should reject clearing bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        missing_group_bound_proxy_error_message("bound-node-shunt")
    );
}

#[tokio::test]
async fn update_upstream_account_group_rejects_disabling_node_shunt_with_unselectable_bindings() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Node Shunt Disable").await;
    set_test_account_group_name(&state.pool, account_id, Some("disable-node-shunt")).await;
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "disable-node-shunt",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec!["stale-node".to_string()],
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath("disable-node-shunt".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: Some(false),
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: None,
        }),
    )
    .await
    .expect_err("disabling node shunt should revalidate unselectable bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "select at least one available proxy node or clear bindings before saving"
    );
}

#[tokio::test]
async fn update_upstream_account_group_rejects_invalid_routing_policy_enums() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        AxumPath("invalid-policy".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                allow_cut_out: OptionalField::Missing,
                allow_cut_in: OptionalField::Missing,
                priority_tier: OptionalField::Value("urgent".to_string()),
                fast_mode_rewrite_mode: OptionalField::Value("keep_original".to_string()),
                image_tool_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Missing,
                status_change_reasons: None,
                timeouts: None,
            }),
        }),
    )
    .await
    .expect_err("invalid routing policy enum should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "priorityTier must be one of: primary, normal, fallback, no_new"
    );
}

#[tokio::test]
async fn update_upstream_account_group_clears_available_models_when_policy_submits_inherit() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "clear-model-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_available_models_json = '["gpt-5.5"]'
            WHERE group_name = 'clear-model-group'
            "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed group available models");

    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath("clear-model-group".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                allow_cut_out: OptionalField::Missing,
                allow_cut_in: OptionalField::Missing,
                priority_tier: OptionalField::Missing,
                fast_mode_rewrite_mode: OptionalField::Missing,
                image_tool_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Null,
                status_change_reasons: None,
                timeouts: None,
            }),
        }),
    )
    .await
    .expect("clear group available models");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT policy_available_models_json
            FROM pool_upstream_account_group_notes
            WHERE group_name = 'clear-model-group'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load cleared group policy");
    assert_eq!(stored, None);
}

#[tokio::test]
async fn update_upstream_account_group_preserves_available_models_when_field_is_omitted() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "preserve-model-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_available_models_json = '["gpt-5.5"]'
            WHERE group_name = 'preserve-model-group'
            "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed group available models");

    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath("preserve-model-group".to_string()),
        Json(UpdateUpstreamAccountGroupRequest {
            note: None,
            bound_proxy_keys: None,
            node_shunt_enabled: None,
            single_account_rotation_enabled: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            concurrency_limit: None,
            routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                allow_cut_out: OptionalField::Missing,
                allow_cut_in: OptionalField::Missing,
                priority_tier: OptionalField::Value("primary".to_string()),
                fast_mode_rewrite_mode: OptionalField::Missing,
                image_tool_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Missing,
                status_change_reasons: None,
                timeouts: None,
            }),
        }),
    )
    .await
    .expect("preserve omitted group available models");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT policy_available_models_json
            FROM pool_upstream_account_group_notes
            WHERE group_name = 'preserve-model-group'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load preserved group policy");
    assert_eq!(stored.as_deref(), Some("[\"gpt-5.5\"]"));
}

#[tokio::test]
async fn create_api_key_account_persists_node_shunt_for_existing_multi_account_group() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 12,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let secondary_proxy_key = {
        let mut manager = state.forward_proxy.lock().await;
        let settings = ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:18080".to_string()],
            ..Default::default()
        };
        manager.apply_settings(settings);
        manager
            .binding_nodes()
            .into_iter()
            .find(|node| node.key != FORWARD_PROXY_DIRECT_KEY)
            .map(|node| node.key)
            .expect("secondary proxy binding key")
    };
    let existing_account_id = insert_api_key_account(&state.pool, "Existing Shared Group").await;
    set_test_account_group_name(&state.pool, existing_account_id, Some("shared-write-group")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "shared-write-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![
                FORWARD_PROXY_DIRECT_KEY.to_string(),
                secondary_proxy_key.clone(),
            ],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save shared group metadata");
    drop(conn);

    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Created Shared Group Account",
        "apiKey": "sk-created-shared-group",
        "groupName": "shared-write-group",
        "groupBoundProxyKeys": [
            FORWARD_PROXY_DIRECT_KEY,
            secondary_proxy_key
        ],
        "groupNodeShuntEnabled": true
    }))
    .expect("deserialize api key create request");
    let Json(detail) =
        create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("create api key account in existing group");

    assert_eq!(
        detail.summary.group_name.as_deref(),
        Some("shared-write-group")
    );
    let metadata = load_group_metadata(&state.pool, Some("shared-write-group"))
        .await
        .expect("load shared group metadata");
    assert!(metadata.node_shunt_enabled);

    server.abort();
}

#[tokio::test]
async fn create_api_key_account_reports_conflict_when_post_create_sync_lacks_node_shunt_slot() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let occupying_account_id =
        insert_api_key_account(&state.pool, "Existing Node Shunt Occupant").await;
    set_test_account_group_name(
        &state.pool,
        occupying_account_id,
        Some("node-shunt-create-blocked"),
    )
    .await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-create-blocked",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: true,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save node shunt create metadata");
    drop(conn);

    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Blocked Node Shunt Create",
        "apiKey": "sk-blocked-node-shunt-create",
        "groupName": "node-shunt-create-blocked"
    }))
    .expect("deserialize blocked api key create request");
    let err = create_api_key_account_inner(state, payload)
        .await
        .expect_err("create api key account should fail without a node slot");

    assert_eq!(err.0, StatusCode::CONFLICT);
    assert_eq!(err.1, group_node_shunt_unassigned_error_message());
}

#[tokio::test]
async fn update_upstream_account_persists_node_shunt_for_existing_multi_account_group() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Shared Group Target").await;
    let sibling_account_id = insert_api_key_account(&state.pool, "Shared Group Sibling").await;
    for grouped_account_id in [account_id, sibling_account_id] {
        set_test_account_group_name(&state.pool, grouped_account_id, Some("shared-update-group"))
            .await;
    }

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "shared-update-group",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: test_required_group_bound_proxy_keys(),
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 0,
        },
    )
    .await
    .expect("save shared update group metadata");
    drop(conn);

    let Json(detail) = update_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(account_id),
        Json(UpdateUpstreamAccountRequest {
            display_name: None,
            email: OptionalField::Missing,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: Some(true),
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            upstream_base_url: OptionalField::Missing,
            bound_proxy_keys: OptionalField::Missing,
            enabled: None,
            is_mother: None,
            api_key: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            tag_ids: None,
            routing_rule: None,
            ..UpdateUpstreamAccountRequest::default()
        }),
    )
    .await
    .expect("update shared group account");

    assert_eq!(
        detail.summary.group_name.as_deref(),
        Some("shared-update-group")
    );
    let metadata = load_group_metadata(&state.pool, Some("shared-update-group"))
        .await
        .expect("load shared update group metadata");
    assert!(metadata.node_shunt_enabled);
}

#[tokio::test]
async fn resolve_required_group_proxy_binding_for_write_allows_node_shunt_without_selectable_nodes()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let requested_bound_proxy_keys = vec!["stale-node".to_string()];
    let expected_bound_proxy_keys =
        canonicalize_forward_proxy_bound_keys(state.as_ref(), &requested_bound_proxy_keys)
            .await
            .expect("canonicalize bound proxy keys");
    let binding = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        Some("write-node-shunt".to_string()),
        Some(requested_bound_proxy_keys),
        Some(true),
    )
    .await
    .expect("node shunt writes should not require selectable nodes");

    assert_eq!(binding.group_name, "write-node-shunt");
    assert_eq!(binding.bound_proxy_keys, expected_bound_proxy_keys);
    assert!(binding.node_shunt_enabled);
}

#[tokio::test]
async fn resolve_required_group_proxy_binding_for_write_rejects_empty_bindings_for_node_shunt() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let err = resolve_required_group_proxy_binding_for_write(
        state.as_ref(),
        Some("write-node-shunt".to_string()),
        Some(vec![]),
        Some(true),
    )
    .await
    .expect_err("node shunt writes should reject empty bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        missing_group_bound_proxy_error_message("write-node-shunt")
    );
}

#[tokio::test]
async fn build_imported_oauth_validation_response_returns_assignment_errors() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let binding = ResolvedRequiredGroupProxyBinding {
        group_name: "import-group".to_string(),
        bound_proxy_keys: vec!["stale-node".to_string()],
        node_shunt_enabled: true,
    };
    let items = vec![ImportOauthCredentialFileRequest {
        source_id: "source-1".to_string(),
        file_name: "alpha.json".to_string(),
        content: json!({
            "type": "codex",
            "email": "alpha@duckmail.sbs",
            "account_id": "acct_alpha",
            "expired": "2026-03-20T00:00:00Z",
            "access_token": "access-token",
            "refresh_token": "refresh-token",
            "id_token": test_id_token(
                "alpha@duckmail.sbs",
                Some("acct_alpha"),
                Some("user_alpha"),
                Some("team"),
            ),
        })
        .to_string(),
    }];

    state.pool.close().await;

    let error = build_imported_oauth_validation_response(state.as_ref(), &items, &binding)
        .await
        .expect_err("assignment build failures should not be swallowed");
    assert!(
        error.to_string().contains("closed") || error.to_string().contains("pool"),
        "unexpected error: {error:#}"
    );
}

#[tokio::test]
async fn create_bulk_upstream_account_sync_job_reuses_existing_running_job() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let snapshot = BulkUpstreamAccountSyncSnapshot {
        job_id: "running-job".to_string(),
        status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
        rows: vec![BulkUpstreamAccountSyncRow {
            account_id: 5,
            display_name: "Existing OAuth".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING.to_string(),
            detail: None,
        }],
    };
    let counts = compute_bulk_upstream_account_sync_counts(&snapshot.rows);
    state
        .upstream_accounts
        .insert_bulk_sync_job(
            snapshot.job_id.clone(),
            Arc::new(BulkUpstreamAccountSyncJob::new(snapshot.clone())),
        )
        .await;

    let response = create_bulk_upstream_account_sync_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(BulkUpstreamAccountSyncJobRequest {
            account_ids: vec![9, 11],
        }),
    )
    .await
    .expect("reuse running bulk sync job")
    .0;

    assert_eq!(response.job_id, "running-job");
    assert_eq!(response.snapshot.job_id, "running-job");
    assert_eq!(response.snapshot.rows.len(), 1);
    assert_eq!(response.snapshot.rows[0].account_id, 5);
    assert_eq!(response.counts.total, counts.total);
    assert_eq!(response.counts.completed, counts.completed);
    assert_eq!(state.upstream_accounts.bulk_sync_jobs.lock().await.len(), 1);
}

#[tokio::test]
async fn finish_bulk_sync_job_completed_exposes_completed_status_in_events_and_response() {
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(
        BulkUpstreamAccountSyncSnapshot {
            job_id: "job-completed".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
            rows: vec![BulkUpstreamAccountSyncRow {
                account_id: 5,
                display_name: "Existing OAuth".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED.to_string(),
                detail: None,
            }],
        },
    ));
    let mut receiver = job.broadcaster.subscribe();

    finish_bulk_upstream_account_sync_job_completed(&job).await;

    match receiver.recv().await.expect("completed event") {
        BulkUpstreamAccountSyncJobEvent::Completed(payload) => {
            assert_eq!(
                payload.snapshot.status,
                BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
            );
            assert_eq!(payload.counts.completed, 1);
            assert_eq!(payload.counts.failed, 0);
            assert_eq!(payload.counts.skipped, 0);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let response =
        build_bulk_upstream_account_sync_job_response("job-completed".to_string(), &job).await;
    assert_eq!(
        response.snapshot.status,
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED
    );
}

#[tokio::test]
async fn finish_bulk_sync_job_failed_exposes_failed_status_in_events_and_response() {
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(
        BulkUpstreamAccountSyncSnapshot {
            job_id: "job-failed".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
            rows: vec![BulkUpstreamAccountSyncRow {
                account_id: 5,
                display_name: "Existing OAuth".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED.to_string(),
                detail: Some("upstream rejected".to_string()),
            }],
        },
    ));
    let mut receiver = job.broadcaster.subscribe();

    finish_bulk_upstream_account_sync_job_failed(&job, "job failed".to_string()).await;

    match receiver.recv().await.expect("failed event") {
        BulkUpstreamAccountSyncJobEvent::Failed(payload) => {
            assert_eq!(
                payload.snapshot.status,
                BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
            );
            assert_eq!(payload.counts.failed, 1);
            assert_eq!(payload.error, "job failed");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let response =
        build_bulk_upstream_account_sync_job_response("job-failed".to_string(), &job).await;
    assert_eq!(
        response.snapshot.status,
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED
    );
}

#[tokio::test]
async fn finish_bulk_sync_job_cancelled_exposes_cancelled_status_in_events_and_response() {
    let job = Arc::new(BulkUpstreamAccountSyncJob::new(
        BulkUpstreamAccountSyncSnapshot {
            job_id: "job-cancelled".to_string(),
            status: BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING.to_string(),
            rows: vec![BulkUpstreamAccountSyncRow {
                account_id: 5,
                display_name: "Existing OAuth".to_string(),
                status: BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED.to_string(),
                detail: Some("disabled accounts cannot be synced".to_string()),
            }],
        },
    ));
    let mut receiver = job.broadcaster.subscribe();

    finish_bulk_upstream_account_sync_job_cancelled(&job).await;

    match receiver.recv().await.expect("cancelled event") {
        BulkUpstreamAccountSyncJobEvent::Cancelled(payload) => {
            assert_eq!(
                payload.snapshot.status,
                BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
            );
            assert_eq!(payload.counts.skipped, 1);
            assert_eq!(payload.counts.completed, 1);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let response =
        build_bulk_upstream_account_sync_job_response("job-cancelled".to_string(), &job).await;
    assert_eq!(
        response.snapshot.status,
        BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED
    );
}

#[test]
fn imported_snapshot_is_exhausted_when_any_limit_is_full_or_credits_are_empty() {
    let primary_exhausted = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "limit-primary".to_string(),
        limit_name: Some("Primary".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 100.0,
            window_duration_mins: 300,
            resets_at: Some("2026-03-20T05:00:00Z".to_string()),
        }),
        secondary: None,
        credits: None,
    };
    assert!(imported_snapshot_is_exhausted(&primary_exhausted));

    let credits_exhausted = NormalizedUsageSnapshot {
        plan_type: Some("team".to_string()),
        limit_id: "limit-credits".to_string(),
        limit_name: Some("Credits".to_string()),
        primary: Some(NormalizedUsageWindow {
            used_percent: 42.0,
            window_duration_mins: 300,
            resets_at: Some("2026-03-20T05:00:00Z".to_string()),
        }),
        secondary: Some(NormalizedUsageWindow {
            used_percent: 12.0,
            window_duration_mins: 10_080,
            resets_at: Some("2026-03-27T00:00:00Z".to_string()),
        }),
        credits: Some(CreditsSnapshot {
            has_credits: true,
            unlimited: false,
            balance: Some("0".to_string()),
        }),
    };
    assert!(imported_snapshot_is_exhausted(&credits_exhausted));
}

#[tokio::test]
async fn resolve_pool_account_upstream_base_url_only_overrides_api_key_accounts() {
    let _upstream_lock = crate::oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
        .lock()
        .await;
    crate::oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;

    fn build_row(kind: &str, upstream_base_url: Option<&str>) -> UpstreamAccountRow {
        UpstreamAccountRow {
            id: 1,
            kind: kind.to_string(),
            provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
            display_name: "Test".to_string(),
            group_name: None,
            bound_proxy_keys_json: None,
            is_mother: 0,
            note: None,
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE.to_string(),
            enabled: 1,
            external_client_id: None,
            external_source_account_id: None,
            email: None,
            verified_email: None,
            chatgpt_account_id: None,
            chatgpt_user_id: None,
            plan_type: None,
            plan_type_observed_at: None,
            masked_api_key: None,
            encrypted_credentials: None,
            has_refresh_token: Some(1),
            token_expires_at: None,
            last_refreshed_at: None,
            last_synced_at: None,
            last_successful_sync_at: None,
            last_activity_at: None,
            last_error: None,
            last_error_at: None,
            last_action: None,
            last_action_source: None,
            last_action_reason_code: None,
            last_action_reason_message: None,
            policy_responses_first_byte_timeout_secs: None,
            policy_compact_first_byte_timeout_secs: None,
            policy_image_first_byte_timeout_secs: None,
            policy_responses_stream_timeout_secs: None,
            policy_compact_stream_timeout_secs: None,
            last_action_http_status: None,
            last_action_invoke_id: None,
            last_action_at: None,
            last_selected_at: None,
            last_route_failure_at: None,
            last_route_failure_kind: None,
            cooldown_until: None,
            consecutive_route_failures: 0,
            temporary_route_failure_streak_started_at: None,
            compact_support_status: None,
            compact_support_observed_at: None,
            compact_support_reason: None,
            response_endpoint_capability: None,
            response_endpoint_capability_observed_at: None,
            response_endpoint_capability_reason: None,
            policy_response_endpoint_capability_override: None,
            image_endpoint_capability: None,
            image_endpoint_capability_observed_at: None,
            image_endpoint_capability_reason: None,
            policy_image_endpoint_capability_override: None,
            response_image_tool_capability: None,
            response_image_tool_capability_observed_at: None,
            response_image_tool_capability_reason: None,
            policy_response_image_tool_capability_override: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            policy_allow_cut_out: None,
            policy_allow_cut_in: None,
            policy_priority_tier: None,
            policy_fast_mode_rewrite_mode: None,
            policy_image_tool_rewrite_mode: None,
            policy_request_compression_algorithm: None,
            policy_concurrency_limit: None,
            policy_upstream_429_retry_enabled: None,
            policy_upstream_429_max_retries: None,
            policy_available_models_json: None,
            policy_status_change_upstream_http_401: None,
            policy_status_change_upstream_http_402: None,
            policy_status_change_upstream_http_403: None,
            policy_status_change_reauth_required: None,
            policy_status_change_upstream_http_429_rate_limit: None,
            policy_status_change_upstream_http_429_quota_exhausted: None,
            policy_status_change_usage_snapshot_exhausted: None,
            policy_status_change_quota_still_exhausted: None,
            policy_status_change_transport_failure: None,
            policy_status_change_upstream_server_overloaded: None,
            policy_status_change_upstream_http_5xx: None,
            upstream_base_url: upstream_base_url.map(str::to_string),
            created_at: "2026-03-15T00:00:00Z".to_string(),
            updated_at: "2026-03-15T00:00:00Z".to_string(),
        }
    }

    let global = Url::parse("https://api.openai.com/").expect("global upstream base url");
    let override_url = "https://proxy.example.com/gateway";
    crate::oauth_bridge::set_test_oauth_codex_upstream_base_url(
        Url::parse("https://chatgpt.com/backend-api/codex").expect("oauth codex base"),
    )
    .await;

    let oauth_row = build_row(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX, Some(override_url));
    let oauth_resolved = resolve_pool_account_upstream_base_url(&oauth_row, &global)
        .expect("resolve oauth upstream base url");
    assert_eq!(
        oauth_resolved.as_str(),
        "https://chatgpt.com/backend-api/codex"
    );

    let api_key_row = build_row(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX, Some(override_url));
    let api_key_resolved = resolve_pool_account_upstream_base_url(&api_key_row, &global)
        .expect("resolve api key upstream base url");
    assert_eq!(
        api_key_resolved.as_str(),
        "https://proxy.example.com/gateway"
    );
}

#[test]
fn parse_chatgpt_jwt_claims_extracts_identity_fields() {
    let payload = json!({
        "email": "user@example.com",
        "https://api.openai.com/auth": {
            "chatgpt_plan_type": "pro",
            "chatgpt_user_id": "user_123",
            "chatgpt_account_id": "org_123"
        }
    });
    let encoded = URL_SAFE_NO_PAD.encode(b"{}");
    let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
    let token = format!("{encoded}.{body}.{encoded}");
    let claims = parse_chatgpt_jwt_claims(&token).expect("parse token");
    assert_eq!(claims.email.as_deref(), Some("user@example.com"));
    assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("pro"));
    assert_eq!(claims.chatgpt_user_id.as_deref(), Some("user_123"));
    assert_eq!(claims.chatgpt_account_id.as_deref(), Some("org_123"));
}

#[test]
fn build_usage_endpoint_url_preserves_backend_api_prefix() {
    let base = Url::parse("https://chatgpt.com/backend-api").expect("chatgpt base");
    let resolved = build_usage_endpoint_url(&base).expect("resolved usage url");
    assert_eq!(
        resolved.as_str(),
        "https://chatgpt.com/backend-api/wham/usage"
    );

    let base_with_slash =
        Url::parse("https://chatgpt.com/backend-api/").expect("chatgpt base with slash");
    let resolved_with_slash =
        build_usage_endpoint_url(&base_with_slash).expect("resolved usage url");
    assert_eq!(
        resolved_with_slash.as_str(),
        "https://chatgpt.com/backend-api/wham/usage"
    );
}

#[test]
fn normalize_usage_snapshot_reads_windows_and_resets() {
    let payload = json!({
        "planType": "pro",
        "rateLimit": {
            "primaryWindow": {
                "usedPercent": 42,
                "windowDurationMins": 300,
                "resetsAt": 1771322400
            },
            "secondaryWindow": {
                "usedPercent": 18.5,
                "windowDurationMins": 10080,
                "resetsAt": 1771927200
            }
        },
        "credits": {
            "hasCredits": true,
            "unlimited": false,
            "balance": "9.99"
        }
    });
    let snapshot = normalize_usage_snapshot(&payload).expect("normalize snapshot");
    assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
    assert_eq!(
        snapshot.primary.as_ref().map(|value| value.used_percent),
        Some(42.0)
    );
    assert_eq!(
        snapshot.secondary.as_ref().map(|value| value.used_percent),
        Some(18.5)
    );
    assert_eq!(
        snapshot
            .credits
            .as_ref()
            .and_then(|value| value.balance.clone())
            .as_deref(),
        Some("9.99")
    );
}

pub(crate) fn usage_snapshot_test_config(base_url: &str, user_agent: &str) -> AppConfig {
    AppConfig {
        openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
        database_path: PathBuf::from(":memory:"),
        poll_interval: Duration::from_secs(10),
        request_timeout: Duration::from_secs(5),
        pool_upstream_responses_attempt_timeout: Duration::from_secs(
            DEFAULT_POOL_UPSTREAM_RESPONSES_ATTEMPT_TIMEOUT_SECS,
        ),
        pool_upstream_responses_total_timeout: Duration::from_secs(
            DEFAULT_POOL_UPSTREAM_RESPONSES_TOTAL_TIMEOUT_SECS,
        ),
        openai_proxy_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_compact_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_image_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_IMAGE_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_request_read_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
        ),
        openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        openai_proxy_websocket_enabled: DEFAULT_OPENAI_PROXY_WEBSOCKET_ENABLED,
        openai_proxy_upstream_websocket_default_enabled:
            DEFAULT_OPENAI_PROXY_UPSTREAM_WEBSOCKET_DEFAULT_ENABLED,
        openai_proxy_encrypted_session_owner_routing_enabled:
            DEFAULT_OPENAI_PROXY_ENCRYPTED_SESSION_OWNER_ROUTING_ENABLED,
        proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
        proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
        proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
        proxy_raw_dir: PathBuf::from("target/proxy-raw-tests"),
        proxy_raw_compression: DEFAULT_PROXY_RAW_COMPRESSION,
        proxy_raw_immediate_gzip_bytes: DEFAULT_PROXY_RAW_IMMEDIATE_GZIP_BYTES,
        proxy_raw_hot_secs: DEFAULT_PROXY_RAW_HOT_SECS,
        xray_binary: DEFAULT_XRAY_BINARY.to_string(),
        xray_runtime_dir: PathBuf::from("target/xray-forward-tests"),
        forward_proxy_algo: ForwardProxyAlgo::V1,
        max_parallel_polls: 2,
        shared_connection_parallelism: 1,
        http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
        cors_allowed_origins: Vec::new(),
        list_limit_max: 100,
        user_agent: user_agent.to_string(),
        static_dir: None,
        public_origin: None,
        retention_enabled: DEFAULT_RETENTION_ENABLED,
        retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
        retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
        retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
        retention_catchup_budget: Duration::from_secs(DEFAULT_RETENTION_CATCHUP_BUDGET_SECS),
        archive_dir: PathBuf::from("target/archive-tests"),
        codex_invocation_archive_layout: DEFAULT_CODEX_INVOCATION_ARCHIVE_LAYOUT,
        codex_invocation_archive_segment_granularity:
            DEFAULT_CODEX_INVOCATION_ARCHIVE_SEGMENT_GRANULARITY,
        invocation_archive_codec: DEFAULT_INVOCATION_ARCHIVE_CODEC,
        invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
        invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
        invocation_archive_ttl_days: DEFAULT_INVOCATION_ARCHIVE_TTL_DAYS,
        forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        pool_upstream_request_attempts_retention_days:
            DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_RETENTION_DAYS,
        pool_upstream_request_attempts_archive_ttl_days:
            DEFAULT_POOL_UPSTREAM_REQUEST_ATTEMPTS_ARCHIVE_TTL_DAYS,
        quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        upstream_accounts_oauth_client_id: DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID.to_string(),
        upstream_accounts_oauth_issuer: Url::parse(DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER)
            .expect("valid oauth issuer"),
        upstream_accounts_usage_base_url: Url::parse(base_url).expect("valid usage base url"),
        upstream_accounts_login_session_ttl: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS,
        ),
        upstream_accounts_sync_interval: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS,
        ),
        upstream_accounts_refresh_lead_time: Duration::from_secs(
            DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS,
        ),
        upstream_accounts_history_retention_days: DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS,
        upstream_accounts_kaisoumail: None,
    }
}

#[tokio::test]
async fn fetch_usage_snapshot_retries_with_browser_user_agent() {
    #[derive(Clone)]
    struct UsageSnapshotTestState {
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn handler(
        State(state): State<UsageSnapshotTestState>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        state.requests.lock().await.push(user_agent.clone());
        if user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
            (
                StatusCode::OK,
                json!({
                    "planType": "pro",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 12,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        } else {
            (
                StatusCode::FORBIDDEN,
                json!({ "detail": "blocked user agent" }).to_string(),
            )
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(UsageSnapshotTestState {
            requests: requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });

    let client = Client::builder().build().expect("client");
    let config = usage_snapshot_test_config(
        &format!("http://{addr}/backend-api"),
        "codex-vibe-monitor/0.2.0",
    );

    let snapshot = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect("fetch usage snapshot");

    assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));
    let recorded = requests.lock().await.clone();
    assert_eq!(
        recorded,
        vec![
            "codex-vibe-monitor/0.2.0".to_string(),
            UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
        ]
    );

    server.abort();
}

#[tokio::test]
async fn fetch_usage_snapshot_skips_browser_user_agent_retry_for_upstream_rejected_402() {
    #[derive(Clone)]
    struct UsageSnapshotTestState {
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn handler(
        State(state): State<UsageSnapshotTestState>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        state.requests.lock().await.push(user_agent);
        (
            StatusCode::PAYMENT_REQUIRED,
            json!({ "detail": { "code": "deactivated_workspace" } }).to_string(),
        )
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(UsageSnapshotTestState {
            requests: requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });

    let client = Client::builder().build().expect("client");
    let config = usage_snapshot_test_config(
        &format!("http://{addr}/backend-api"),
        "codex-vibe-monitor/0.2.0",
    );

    let err = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect_err("402 upstream rejected should stay terminal");
    assert!(
        err.to_string().contains("402 Payment Required"),
        "expected original 402 error, got: {err:#}"
    );

    let recorded = requests.lock().await.clone();
    assert_eq!(recorded, vec!["codex-vibe-monitor/0.2.0".to_string()]);

    server.abort();
}

#[tokio::test]
async fn fetch_usage_snapshot_retries_browser_user_agent_for_generic_403_upstream_rejected_text() {
    #[derive(Clone)]
    struct UsageSnapshotTestState {
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn handler(
        State(state): State<UsageSnapshotTestState>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let is_browser_user_agent = user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT;
        state.requests.lock().await.push(user_agent);
        if is_browser_user_agent {
            (
                StatusCode::OK,
                json!({
                    "planType": "pro",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 9,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        } else {
            (
                StatusCode::FORBIDDEN,
                "usage endpoint returned 403 Forbidden: upstream rejected request by policy"
                    .to_string(),
            )
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(UsageSnapshotTestState {
            requests: requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });

    let client = Client::builder().build().expect("client");
    let config = usage_snapshot_test_config(
        &format!("http://{addr}/backend-api"),
        "codex-vibe-monitor/0.2.0",
    );

    let snapshot = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect("generic 403 upstream-rejected text should still retry browser user agent");
    assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));

    let recorded = requests.lock().await.clone();
    assert_eq!(
        recorded,
        vec![
            "codex-vibe-monitor/0.2.0".to_string(),
            UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
        ]
    );

    server.abort();
}

#[tokio::test]
async fn fetch_usage_snapshot_retries_browser_user_agent_for_generic_402_pages() {
    #[derive(Clone)]
    struct UsageSnapshotTestState {
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn handler(
        State(state): State<UsageSnapshotTestState>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        state.requests.lock().await.push(user_agent.clone());
        if user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT {
            (
                StatusCode::OK,
                json!({
                    "planType": "pro",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 9,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        } else {
            (StatusCode::PAYMENT_REQUIRED, "Payment Required".to_string())
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(UsageSnapshotTestState {
            requests: requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });

    let client = Client::builder().build().expect("client");
    let config = usage_snapshot_test_config(
        &format!("http://{addr}/backend-api"),
        "codex-vibe-monitor/0.2.0",
    );

    let snapshot = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect("generic 402 should retry with browser user agent");
    assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));

    let recorded = requests.lock().await.clone();
    assert_eq!(
        recorded,
        vec![
            "codex-vibe-monitor/0.2.0".to_string(),
            UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
        ]
    );

    server.abort();
}

#[tokio::test]
async fn fetch_usage_snapshot_retries_browser_user_agent_for_wrapped_upstream_auth_error() {
    #[derive(Clone)]
    struct UsageSnapshotTestState {
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn handler(
        State(state): State<UsageSnapshotTestState>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let is_browser_user_agent = user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT;
        state.requests.lock().await.push(user_agent);
        if is_browser_user_agent {
            (
                StatusCode::OK,
                json!({
                    "planType": "pro",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 9,
                            "windowDurationMins": 300,
                            "resetsAt": 1771322400
                        }
                    }
                })
                .to_string(),
            )
        } else {
            (
                StatusCode::FORBIDDEN,
                "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden"
                    .to_string(),
            )
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(UsageSnapshotTestState {
            requests: requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });

    let client = Client::builder().build().expect("client");
    let config = usage_snapshot_test_config(
        &format!("http://{addr}/backend-api"),
        "codex-vibe-monitor/0.2.0",
    );

    let snapshot = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect("wrapped upstream auth error should still retry browser user agent");
    assert_eq!(snapshot.plan_type.as_deref(), Some("pro"));

    let recorded = requests.lock().await.clone();
    assert_eq!(
        recorded,
        vec![
            "codex-vibe-monitor/0.2.0".to_string(),
            UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
        ]
    );

    server.abort();
}

#[tokio::test]
async fn fetch_usage_snapshot_preserves_terminal_browser_retry_402_for_classification() {
    #[derive(Clone)]
    struct UsageSnapshotTestState {
        requests: Arc<Mutex<Vec<String>>>,
    }

    async fn handler(
        State(state): State<UsageSnapshotTestState>,
        headers: HeaderMap,
    ) -> (StatusCode, String) {
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let is_browser_user_agent = user_agent == UPSTREAM_USAGE_BROWSER_USER_AGENT;
        state.requests.lock().await.push(user_agent);
        if is_browser_user_agent {
            (
                StatusCode::PAYMENT_REQUIRED,
                json!({ "detail": { "code": "deactivated_workspace" } }).to_string(),
            )
        } else {
            (
                StatusCode::BAD_GATEWAY,
                "upstream usage endpoint temporary gateway failure".to_string(),
            )
        }
    }

    let requests = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/backend-api/wham/usage", get(handler))
        .with_state(UsageSnapshotTestState {
            requests: requests.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let addr = listener.local_addr().expect("listener addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test app");
    });

    let client = Client::builder().build().expect("client");
    let config = usage_snapshot_test_config(
        &format!("http://{addr}/backend-api"),
        "codex-vibe-monitor/0.2.0",
    );

    let err = fetch_usage_snapshot(&client, &config, "access-token", Some("acct_test"))
        .await
        .expect_err("terminal browser retry 402 should surface for sync classification");
    let err_text = err.to_string();
    assert!(
        err_text.contains("browser user agent retry failed"),
        "expected retry context in surfaced error, got: {err:#}"
    );
    assert!(
        err_text.contains("402 Payment Required"),
        "expected terminal browser retry 402 to survive to_string(), got: {err:#}"
    );
    assert!(
        err_text.contains("deactivated_workspace"),
        "expected terminal browser retry detail to survive to_string(), got: {err:#}"
    );

    let recorded = requests.lock().await.clone();
    assert_eq!(
        recorded,
        vec![
            "codex-vibe-monitor/0.2.0".to_string(),
            UPSTREAM_USAGE_BROWSER_USER_AGENT.to_string()
        ]
    );

    server.abort();
}

#[test]
fn build_manual_callback_redirect_uri_targets_localhost() {
    let redirect = build_manual_callback_redirect_uri().expect("redirect uri");
    assert!(redirect.starts_with("http://localhost:"));
    assert!(redirect.ends_with("/auth/callback"));
}

#[test]
fn parse_manual_oauth_callback_accepts_expected_redirect() {
    let query = parse_manual_oauth_callback(
        "http://localhost:37891/auth/callback?code=test-code&state=test-state",
        "http://localhost:37891/auth/callback",
    )
    .expect("callback query");
    assert_eq!(query.code.as_deref(), Some("test-code"));
    assert_eq!(query.state.as_deref(), Some("test-state"));
}

#[test]
fn build_oauth_authorize_url_requests_official_scopes_and_audience() {
    let url = build_oauth_authorize_url(
        &Url::parse("https://auth.openai.com").expect("issuer"),
        "client-id",
        "http://localhost:1455/auth/callback",
        "state-token",
        "challenge",
    )
    .expect("build authorize url");
    let parsed = Url::parse(&url).expect("parse authorize url");
    let query = parsed.query_pairs().into_owned().collect::<HashMap<_, _>>();
    let scope = query
        .get("scope")
        .cloned()
        .expect("scope should be present");
    let scope_parts = scope.split_whitespace().collect::<Vec<_>>();

    assert_eq!(
        query.get("audience").map(String::as_str),
        Some(DEFAULT_OAUTH_AUDIENCE)
    );
    assert_eq!(
        query.get("prompt").map(String::as_str),
        Some(DEFAULT_OAUTH_PROMPT)
    );
    assert!(scope_parts.contains(&"openid"));
    assert!(scope_parts.contains(&"profile"));
    assert!(scope_parts.contains(&"email"));
    assert!(scope_parts.contains(&"offline_access"));
    assert_eq!(scope_parts.len(), 4);
}

#[test]
fn is_reauth_error_requires_explicit_invalidated_signal() {
    assert!(is_reauth_error(&anyhow!(
        "OAuth token endpoint returned 400: invalid_grant"
    )));
    assert!(is_reauth_error(&anyhow!(
        "Authentication token has been invalidated, please sign in again"
    )));
    assert!(!is_reauth_error(&anyhow!(
        "usage endpoint returned 401: Missing scopes: api.responses.write"
    )));
    assert!(!is_reauth_error(&anyhow!(
        "pool upstream responded with 403: You have insufficient permissions for this operation."
    )));
}

pub(crate) async fn test_pool() -> SqlitePool {
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
    let proxy_raw_async_writer_limit = proxy_raw_async_writer_limit(&config);
    let pool = test_pool().await;
    Arc::new(AppState {
        config,
        sqlite_batch_writer: SqliteBatchWriter::spawn_for_test(),
        pool_account_selection_runtime: Arc::new(PoolAccountSelectionRuntime::default()),
        proxy_runtime_invocations: Arc::new(ProxyRuntimeInvocationStore::default()),
        pool,
        oauth_installation_seed: [0_u8; 32],
        http_clients,
        broadcaster,
        subscription_hub: Arc::new(crate::SubscriptionHub::new()),
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache {
            summaries: HashMap::new(),
            quota: None,
        })),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        proxy_summary_quota_broadcast_handle: Arc::new(Mutex::new(Vec::new())),
        dashboard_activity_live_broadcast_seq: Arc::new(AtomicU64::new(0)),
        dashboard_activity_live_broadcast_running: Arc::new(AtomicBool::new(false)),
        process_started_at_utc: chrono::Utc::now(),
        dashboard_network_speed_cache: Arc::new(
            crate::dashboard_network_speed::DashboardNetworkSpeedCache::new(chrono::Utc::now()),
        ),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore: Arc::new(Semaphore::new(4)),
        proxy_request_in_flight: Arc::new(AtomicUsize::new(0)),
        proxy_raw_async_semaphore: Arc::new(Semaphore::new(proxy_raw_async_writer_limit)),
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
        })),
        maintenance_stats_cache: Arc::new(Mutex::new(StatsMaintenanceCacheState::default())),
        system_status_cache: Arc::new(Mutex::new(SystemStatusCacheState::default())),
        pool_routing_reservations: Arc::new(std::sync::Mutex::new(HashMap::new())),
        pool_routing_runtime_cache: Arc::new(Mutex::new(None)),
        pool_live_attempt_ids: Arc::new(std::sync::Mutex::new(HashSet::new())),
        pool_group_429_retry_delay_override: None,
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
