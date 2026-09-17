use super::*;
use serde_json::json;
use std::time::Duration;

struct SyncFailureExpectation {
    status: &'static str,
    reason_code: &'static str,
    http_status: Option<i64>,
    error_contains: &'static str,
    route_failure_kind: Option<&'static str>,
    health_status: &'static str,
    work_status: &'static str,
    usage_requests: usize,
    token_requests: usize,
}

async fn assert_node_shunt_sync_success(state: &AppState, account_id: i64) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load node shunt account after sync")
        .expect("node shunt account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    let detail = load_upstream_account_detail_with_actual_usage(state, account_id)
        .await
        .expect("load node shunt account detail")
        .expect("node shunt account detail exists");
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );
}

async fn assert_node_shunt_account_active(state: &AppState, account_id: i64) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load node shunt account after sync")
        .expect("node shunt account still exists");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_successful_sync_at.is_some());
}

fn assert_bulk_sync_completed(terminal: BulkUpstreamAccountSyncTerminalEvent, account_id: i64) {
    let BulkUpstreamAccountSyncTerminalEvent::Completed(payload) = terminal else {
        panic!("bulk sync job should complete successfully");
    };
    assert_eq!(payload.counts.total, 1);
    assert_eq!(payload.counts.completed, 1);
    assert_eq!(payload.counts.failed, 0);
    assert_eq!(payload.snapshot.rows.len(), 1);
    assert_eq!(
        payload.snapshot.rows[0].status,
        BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED
    );
    assert_eq!(payload.snapshot.rows[0].account_id, account_id);
}

async fn assert_node_shunt_account_is_unassigned(
    state: &AppState,
    occupying_account_id: i64,
    queued_account_id: i64,
) {
    let assignments = build_upstream_account_node_shunt_assignments(state)
        .await
        .expect("build node shunt assignments");
    assert_eq!(
        assignments
            .account_proxy_keys
            .get(&occupying_account_id)
            .map(String::as_str),
        Some(FORWARD_PROXY_DIRECT_KEY),
    );
    assert!(
        !assignments
            .account_proxy_keys
            .contains_key(&queued_account_id)
    );
}

async fn seed_node_shunt_selected_at(state: &AppState, values: &[(i64, String)]) {
    for (account_id, selected_at) in values {
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET last_selected_at = ?2,
                    updated_at = ?2
                WHERE id = ?1
                "#,
        )
        .bind(account_id)
        .bind(selected_at)
        .execute(&state.pool)
        .await
        .expect("seed last_selected_at");
    }
}

async fn assert_node_shunt_list_filtering(
    state: &AppState,
    occupying_account_id: i64,
    queued_account_id: i64,
) {
    let mut all_items = load_upstream_account_summaries_for_query(
        &state.pool,
        &state.config,
        &ListUpstreamAccountsQuery::default(),
    )
    .await
    .expect("load upstream account summaries");
    enrich_node_shunt_routing_block_reasons(state, &mut all_items)
        .await
        .expect("enrich node shunt routing block reasons");

    let idle_filters = normalize_upstream_account_list_filters(&ListUpstreamAccountsQuery {
        work_status: vec![UPSTREAM_ACCOUNT_WORK_STATUS_IDLE.to_string()],
        ..ListUpstreamAccountsQuery::default()
    });
    let idle_items = filter_upstream_account_summaries(all_items.clone(), &idle_filters);
    let idle_metrics = build_upstream_account_list_metrics(&idle_items);
    assert_eq!(idle_items.len(), 1);
    assert_eq!(idle_metrics.total, 1);
    assert_eq!(idle_items[0].id, queued_account_id);
    assert_eq!(idle_items[0].work_status, UPSTREAM_ACCOUNT_WORK_STATUS_IDLE);
    assert_eq!(
        idle_items[0].routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );

    let working_filters = normalize_upstream_account_list_filters(&ListUpstreamAccountsQuery {
        work_status: vec![UPSTREAM_ACCOUNT_WORK_STATUS_WORKING.to_string()],
        ..ListUpstreamAccountsQuery::default()
    });
    let working_items = filter_upstream_account_summaries(all_items, &working_filters);
    assert_eq!(working_items.len(), 1);
    assert_eq!(working_items[0].id, occupying_account_id);
    assert!(
        working_items
            .iter()
            .all(|item| item.id != queued_account_id)
    );
}

async fn assert_oauth_sync_failure_state(
    state: &AppState,
    account_id: i64,
    usage_requests: &AtomicUsize,
    token_requests: &AtomicUsize,
    expected: SyncFailureExpectation,
) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync failure")
        .expect("oauth row exists after sync failure");
    assert_eq!(after.status, expected.status);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(expected.reason_code)
    );
    assert_eq!(after.last_action_http_status, expected.http_status);
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains(expected.error_contains))
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        expected.route_failure_kind
    );

    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, expected.status);
    assert_eq!(summary.display_status, expected.status);
    assert_eq!(summary.health_status, expected.health_status);
    assert_eq!(summary.work_status, expected.work_status);
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);

    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(detail.summary.display_status, expected.status);
    assert_eq!(detail.summary.work_status, expected.work_status);
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        usage_requests.load(Ordering::SeqCst),
        expected.usage_requests
    );
    assert_eq!(
        token_requests.load(Ordering::SeqCst),
        expected.token_requests
    );
}

#[tokio::test]
pub(crate) async fn maintenance_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node()
 {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Maintenance OAuth",
        "occupying-maintenance@example.com",
        "org_occupying_maintenance",
        "user_occupying_maintenance",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Maintenance OAuth",
        "queued-maintenance@example.com",
        "org_queued_maintenance",
        "user_queued_maintenance",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-maint")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-maint")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-maint",
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
    .expect("save node shunt maintenance metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    let outcome = state
        .upstream_accounts
        .account_ops
        .run_maintenance_sync(state.clone(), queued_account_id)
        .await
        .expect("maintenance sync should execute via shared bound-node probe");
    assert!(matches!(outcome, MaintenanceDispatchOutcome::Executed));

    assert_node_shunt_sync_success(state.as_ref(), queued_account_id).await;

    server.abort();
}

#[tokio::test]
pub(crate) async fn bulk_sync_allows_group_node_shunt_unassigned_account_to_probe_bound_node() {
    let (state, queued_account_id, server) = seed_bulk_node_shunt_probe_fixture().await;
    let response = create_bulk_upstream_account_sync_job(
        State(state.clone()),
        HeaderMap::new(),
        Json(BulkUpstreamAccountSyncJobRequest {
            account_ids: vec![queued_account_id],
        }),
    )
    .await
    .expect("create bulk sync job")
    .0;
    let job = state
        .upstream_accounts
        .get_bulk_sync_job(&response.job_id)
        .await
        .expect("bulk sync job exists");
    let terminal = timeout(Duration::from_secs(15), async {
        loop {
            if let Some(terminal) = job.terminal_event.lock().await.clone() {
                return terminal;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bulk sync job should finish within timeout");
    assert_bulk_sync_completed(terminal, queued_account_id);
    assert_node_shunt_account_active(state.as_ref(), queued_account_id).await;
    server.abort();
}

async fn seed_bulk_node_shunt_probe_fixture() -> (Arc<AppState>, i64, tokio::task::JoinHandle<()>) {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Bulk OAuth",
        "occupying-bulk@example.com",
        "org_occupying_bulk",
        "user_occupying_bulk",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Bulk OAuth",
        "queued-bulk@example.com",
        "org_queued_bulk",
        "user_queued_bulk",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-bulk")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-bulk")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-bulk",
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
    .expect("save node shunt bulk metadata");
    drop(conn);

    seed_hard_unavailable_route_failure(
        &state.pool,
        queued_account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    (state, queued_account_id, server)
}

#[tokio::test]
pub(crate) async fn detail_preserves_group_node_shunt_unassigned_routing_block_reason() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying OAuth",
        "occupying@example.com",
        "org_occupying",
        "user_occupying",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued OAuth",
        "queued@example.com",
        "org_queued",
        "user_queued",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-sync")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-sync")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-sync",
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
    .expect("save node shunt sync metadata");
    drop(conn);

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), queued_account_id)
        .await
        .expect("load queued account detail")
        .expect("queued account detail exists");
    assert_eq!(detail.summary.id, queued_account_id);
    assert_eq!(
        detail.summary.routing_block_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED),
    );
    assert_eq!(
        detail.summary.routing_block_reason_message.as_deref(),
        Some(group_node_shunt_unassigned_error_message()),
    );
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_applies_node_shunt_idle_rewrite_before_filters() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let occupying_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Occupying Filtered OAuth",
        "occupying-filtered@example.com",
        "org_occupying_filtered",
        "user_occupying_filtered",
    )
    .await;
    let queued_account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Queued Filtered OAuth",
        "queued-filtered@example.com",
        "org_queued_filtered",
        "user_queued_filtered",
    )
    .await;

    set_test_account_group_name(&state.pool, occupying_account_id, Some("node-shunt-filter")).await;
    set_test_account_group_name(&state.pool, queued_account_id, Some("node-shunt-filter")).await;

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "node-shunt-filter",
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
    .expect("save node shunt filter metadata");
    drop(conn);

    let occupying_selected_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(3));
    let queued_selected_at = format_utc_iso(Utc::now() - ChronoDuration::minutes(1));
    seed_node_shunt_selected_at(
        state.as_ref(),
        &[
            (occupying_account_id, occupying_selected_at),
            (queued_account_id, queued_selected_at),
        ],
    )
    .await;
    assert_node_shunt_account_is_unassigned(
        state.as_ref(),
        occupying_account_id,
        queued_account_id,
    )
    .await;

    assert_node_shunt_list_filtering(state.as_ref(), occupying_account_id, queued_account_id).await;
}

#[tokio::test]
pub(crate) async fn sync_api_key_account_clears_stale_manual_recovery_marker_on_active_rows() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Active API Key With Stale Marker").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark legacy sync success");
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    sync_api_key_account(&pool, &row, SyncCause::Maintenance)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
}

#[tokio::test]
pub(crate) async fn updating_api_key_reactivates_manually_recoverable_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Recoverable API Key",
        "recoverable-api-key",
        None,
        Some("https://recoverable-api-key.example.com/backend-api/codex"),
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let mut availability = state.pool_routing_availability.subscribe();

    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: Some("sk-live-new".to_string()),
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update api key account");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("manual recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load row after api key update")
        .expect("row exists after api key update");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED)
    );
}

#[tokio::test]
pub(crate) async fn successful_account_sync_publishes_routing_availability_on_recovery() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Sync Recovery Signal",
        "sync-recovery-signal-key",
        None,
        Some("https://sync-recovery-signal.example.com/backend-api/codex"),
    )
    .await;
    set_account_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("temporary sync failure"),
    )
    .await
    .expect("make account unavailable before sync");
    let mut availability = state.pool_routing_availability.subscribe();

    sync_upstream_account_by_id(state.as_ref(), account_id, SyncCause::Manual)
        .await
        .expect("sync should recover the account");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("sync recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let recovered = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load recovered account")
        .expect("recovered account exists");
    assert_eq!(recovered.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(is_account_selectable_for_fresh_assignment(
        &recovered,
        false,
        Utc::now()
    ));
}

#[tokio::test]
pub(crate) async fn imported_oauth_probe_publishes_routing_availability_on_recovery() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Imported OAuth Recovery Signal",
        "import-recovery@example.com",
        "org_import_recovery",
        "user_import_recovery",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let mut availability = state.pool_routing_availability.subscribe();
    let probe = ImportedOauthProbeOutcome {
        token_expires_at: format_utc_iso(Utc::now() + ChronoDuration::days(30)),
        credentials: StoredOauthCredentials {
            access_token: "imported-access-token".to_string(),
            refresh_token: Some("imported-refresh-token".to_string()),
            id_token: test_id_token(
                "import-recovery@example.com",
                Some("org_import_recovery"),
                Some("user_import_recovery"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        claims: ChatgptJwtClaims::default(),
        usage_snapshot: Some(NormalizedUsageSnapshot {
            plan_type: Some("team".to_string()),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: None,
            secondary: None,
            credits: None,
        }),
        maintenance_proxy_snapshot: None,
        exhausted: false,
        usage_snapshot_warning: None,
    };

    apply_imported_oauth_probe_result(state.as_ref(), account_id, &probe)
        .await
        .expect("apply imported OAuth recovery probe");
    tokio::time::timeout(Duration::from_secs(1), availability.changed())
        .await
        .expect("OAuth import recovery should publish routing availability")
        .expect("routing availability signal should stay open");

    let recovered = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load imported OAuth account")
        .expect("imported OAuth account exists");
    assert!(is_account_selectable_for_fresh_assignment(
        &recovered,
        false,
        Utc::now()
    ));

    let unchanged_availability = state.pool_routing_availability.subscribe();
    apply_imported_oauth_probe_result(state.as_ref(), account_id, &probe)
        .await
        .expect("reapply imported OAuth probe to an already routable account");
    assert!(
        !unchanged_availability
            .has_changed()
            .expect("read availability"),
        "an already routable account must not publish a redundant availability event"
    );
}

#[tokio::test]
pub(crate) async fn oauth_sync_keeps_quota_exhausted_accounts_blocked_until_snapshot_recovers() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Exhausted OAuth",
        "exhausted@example.com",
        "org_exhausted",
        "user_exhausted",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Manual)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync")
        .expect("oauth row exists after sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(after.last_error.as_deref(), Some("seed hard unavailable"));
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_ignores_stale_input_row_after_newer_quota_hard_stop() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Stale OAuth Input Row",
        "stale-input@example.com",
        "org_stale_input",
        "user_stale_input",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after stale sync")
        .expect("oauth row exists after stale sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_demotes_active_stale_quota_marker_when_snapshot_is_still_exhausted()
{
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 100,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Legacy Active Exhausted OAuth",
        "legacy-exhausted@example.com",
        "org_legacy_exhausted",
        "user_legacy_exhausted",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    mark_account_sync_success(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark legacy sync success");
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    assert_eq!(row.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        row.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after sync")
        .expect("oauth row exists after sync");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ERROR);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_reactivates_quota_exhausted_account_once_snapshot_recovers() {
    let (base_url, server) = spawn_usage_snapshot_server(
        StatusCode::OK,
        json!({
            "planType": "team",
            "rateLimit": {
                "primaryWindow": {
                    "usedPercent": 42,
                    "windowDurationMins": 300,
                    "resetsAt": 1771322400
                }
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Recovered OAuth Sync",
        "recovered@example.com",
        "org_recovered",
        "user_recovered",
    )
    .await;
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Manual)
        .await
        .expect("sync oauth account");

    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after recovery")
        .expect("oauth row exists after recovery");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
            spawn_sequenced_oauth_sync_server(
                vec![
                    (
                        StatusCode::UNAUTHORIZED,
                        json!({
                            "error": {
                                "message": "Session cookie expired during usage snapshot"
                            }
                        }),
                    ),
                    (
                        StatusCode::FORBIDDEN,
                        json!({
                            "error": {
                                "message": "Authentication token has been invalidated, please sign in again"
                            }
                        }),
                    ),
                ],
                json!({
                    "access_token": "refreshed-access-token",
                    "refresh_token": "refresh-token-rotated",
                    "id_token": test_id_token(
                        "reauth-required@example.com",
                        Some("org_retry_reauth"),
                        Some("user_retry_reauth"),
                        Some("team"),
                    ),
                    "token_type": "Bearer",
                    "expires_in": 3600
                }),
            )
            .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Needs Reauth OAuth",
        "reauth-required@example.com",
        "org_retry_reauth",
        "user_retry_reauth",
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    assert_oauth_sync_failure_state(
        &state,
        account_id,
        &usage_requests,
        &token_requests,
        SyncFailureExpectation {
            status: UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
            reason_code: UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
            http_status: Some(403),
            error_contains: "Authentication token has been invalidated",
            route_failure_kind: Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
            health_status: UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH,
            work_status: UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE,
            usage_requests: 2,
            token_requests: 1,
        },
    )
    .await;
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refreshed oauth row")
        .expect("refreshed oauth row exists");
    let decrypted = decrypt_credentials(
        crypto_key,
        after
            .encrypted_credentials
            .as_deref()
            .expect("encrypted oauth credentials"),
    )
    .expect("decrypt refreshed credentials");
    let StoredCredentials::Oauth(credentials) = decrypted else {
        panic!("unexpected credential kind after refresh")
    };
    assert_eq!(credentials.access_token, "refreshed-access-token");
    assert_eq!(
        credentials.refresh_token.as_deref(),
        Some("refresh-token-rotated")
    );
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing()
 {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-temporary-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "transport-failure@example.com",
                    Some("org_retry_gateway"),
                    Some("user_retry_gateway"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Failure OAuth",
        "transport-failure@example.com",
        "org_retry_gateway",
        "user_retry_gateway",
    )
    .await;
    seed_route_cooldown(
        &state.pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");

    sync_oauth_account(&state, &row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    assert_oauth_sync_failure_state(
        &state,
        account_id,
        &usage_requests,
        &token_requests,
        SyncFailureExpectation {
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            reason_code: "upstream_http_5xx",
            http_status: Some(502),
            error_contains: "usage endpoint returned 502 Bad Gateway",
            route_failure_kind: Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429),
            health_status: UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            work_status: UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED,
            usage_requests: 2,
            token_requests: 1,
        },
    )
    .await;
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load gateway failure row")
        .expect("gateway failure row exists");
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_retry_after_refresh_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![
                (
                    StatusCode::UNAUTHORIZED,
                    json!({
                        "error": {
                            "message": "Session cookie expired during usage snapshot"
                        }
                    }),
                ),
                (
                    StatusCode::BAD_GATEWAY,
                    json!({
                        "error": {
                            "message": "gateway temporarily unavailable"
                        }
                    }),
                ),
            ],
            json!({
                "access_token": "refreshed-quota-preserving-token",
                "refresh_token": "refresh-token-rotated",
                "id_token": test_id_token(
                    "retry-quota-preserve@example.com",
                    Some("org_retry_quota_preserve"),
                    Some("user_retry_quota_preserve"),
                    Some("team"),
                ),
                "token_type": "Bearer",
                "expires_in": 3600
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Retry Gateway Quota Preserve OAuth",
        "retry-quota-preserve@example.com",
        "org_retry_quota_preserve",
        "user_retry_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    assert_oauth_sync_failure_state(
        &state,
        account_id,
        &usage_requests,
        &token_requests,
        SyncFailureExpectation {
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            reason_code: "upstream_http_5xx",
            http_status: Some(502),
            error_contains: "usage endpoint returned 502 Bad Gateway",
            route_failure_kind: Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
            health_status: UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            work_status: UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            usage_requests: 2,
            token_requests: 1,
        },
    )
    .await;
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load gateway failure row")
        .expect("gateway failure row exists");
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_refresh_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, oauth_issuer, usage_requests, token_requests, server) =
        spawn_sequenced_oauth_sync_server(
            vec![(
                StatusCode::UNAUTHORIZED,
                json!({
                    "error": {
                        "message": "Session cookie expired during usage snapshot"
                    }
                }),
            )],
            json!({
                "unexpected": "shape"
            }),
        )
        .await;
    let state = test_app_state_with_usage_and_oauth_base(&usage_base_url, &oauth_issuer).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Refresh Failure Quota Preserve OAuth",
        "refresh-quota-preserve@example.com",
        "org_refresh_quota_preserve",
        "user_refresh_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    assert_oauth_sync_failure_state(
        &state,
        account_id,
        &usage_requests,
        &token_requests,
        SyncFailureExpectation {
            status: UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            reason_code: UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR,
            http_status: None,
            error_contains: "failed to decode OAuth token response",
            route_failure_kind: Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED),
            health_status: UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL,
            work_status: UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED,
            usage_requests: 1,
            token_requests: 1,
        },
    )
    .await;
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load refresh failure row")
        .expect("refresh failure row exists");
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    assert_eq!(usage_requests.load(Ordering::SeqCst), 1);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test]
pub(crate) async fn oauth_sync_direct_fetch_failure_preserves_quota_marker_from_current_db_state() {
    let (usage_base_url, server) = spawn_usage_snapshot_server(
        StatusCode::BAD_GATEWAY,
        json!({
            "error": {
                "message": "gateway temporarily unavailable"
            }
        }),
    )
    .await;
    let state = test_app_state_with_usage_base(&usage_base_url).await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Direct Failure Quota Preserve OAuth",
        "direct-quota-preserve@example.com",
        "org_direct_quota_preserve",
        "user_direct_quota_preserve",
    )
    .await;
    let stale_row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    seed_hard_unavailable_route_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;

    sync_oauth_account(&state, &stale_row, SyncCause::Maintenance)
        .await
        .expect("sync oauth account");

    assert_direct_oauth_sync_failure_state(&state, account_id).await;
    server.abort();
}

async fn assert_direct_oauth_sync_failure_state(state: &AppState, account_id: i64) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after direct fetch failure")
        .expect("oauth row exists after direct fetch failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert!(
        after
            .last_error
            .as_deref()
            .is_some_and(|message| message.contains("502 Bad Gateway"))
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
    assert_eq!(after.last_route_failure_at, after.last_error_at);
    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
}
