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
    let (occupying_account_id, queued_account_id) = seed_node_shunt_filter_accounts(&state).await;

    let assignments = build_upstream_account_node_shunt_assignments(state.as_ref())
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
            .contains_key(&queued_account_id),
        "queued account should be unassigned before list filtering",
    );

    let mut all_items = load_upstream_account_summaries_for_query(
        &state.pool,
        &state.config,
        &ListUpstreamAccountsQuery::default(),
    )
    .await
    .expect("load upstream account summaries");
    enrich_node_shunt_routing_block_reasons(state.as_ref(), &mut all_items)
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
            .all(|item| item.id != queued_account_id),
        "queued account should not remain in working results after node shunt rewrite",
    )
}

async fn seed_node_shunt_filter_accounts(state: &AppState) -> (i64, i64) {
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
    for account_id in [occupying_account_id, queued_account_id] {
        set_test_account_group_name(&state.pool, account_id, Some("node-shunt-filter")).await;
    }
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
    for (account_id, selected_at) in [
        (
            occupying_account_id,
            format_utc_iso(Utc::now() - ChronoDuration::minutes(3)),
        ),
        (
            queued_account_id,
            format_utc_iso(Utc::now() - ChronoDuration::minutes(1)),
        ),
    ] {
        sqlx::query("UPDATE pool_upstream_accounts SET last_selected_at = ?2, updated_at = ?2 WHERE id = ?1")
            .bind(account_id)
            .bind(selected_at)
            .execute(&state.pool)
            .await
            .expect("seed last_selected_at");
    }
    (occupying_account_id, queued_account_id)
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
    oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing_impl().await;
}

async fn oauth_sync_retry_after_refresh_settles_to_needs_reauth_without_stale_syncing_impl() {
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

    assert_needs_reauth_after_refresh(
        &state,
        account_id,
        crypto_key,
        &usage_requests,
        &token_requests,
    )
    .await;
    server.abort();
}

async fn assert_needs_reauth_after_refresh(
    state: &Arc<AppState>,
    account_id: i64,
    crypto_key: &[u8; 32],
    usage_requests: &AtomicUsize,
    token_requests: &AtomicUsize,
) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after retry failure")
        .expect("oauth row exists after retry failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
    );
    assert_eq!(after.last_action_http_status, Some(403));
    assert_eq!(
        after.last_error.as_deref(),
        Some(
            "usage endpoint returned 403 Forbidden: Authentication token has been invalidated, please sign in again"
        )
    );
    assert!(after.last_action_at.is_some());
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
    let summary = build_summary_from_row(
        &after,
        None,
        after.last_activity_at.clone(),
        vec![],
        None,
        0,
        Utc::now(),
    );
    assert_eq!(summary.status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH);
    assert_eq!(
        summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    let detail = load_upstream_account_detail(&state.pool, account_id)
        .await
        .expect("load detail export")
        .expect("detail export exists");
    assert_eq!(
        detail.summary.display_status,
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH
    );
    assert_eq!(
        detail.summary.work_status,
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.reason_code.as_deref()),
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
}

#[tokio::test]
pub(crate) async fn oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing()
 {
    oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing_impl()
        .await;
}

async fn oauth_sync_retry_after_refresh_records_non_auth_terminal_failure_without_stale_syncing_impl()
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

    assert_gateway_failure_after_refresh(&state, account_id, &usage_requests, &token_requests)
        .await;
    server.abort();
}

async fn assert_gateway_failure_after_refresh(
    state: &Arc<AppState>,
    account_id: i64,
    usage_requests: &AtomicUsize,
    token_requests: &AtomicUsize,
) {
    let after = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth row after gateway failure")
        .expect("oauth row exists after gateway failure");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        after.last_action_reason_code.as_deref(),
        Some("upstream_http_5xx")
    );
    assert_eq!(after.last_action_http_status, Some(502));
    assert_eq!(
        after.last_error.as_deref(),
        Some("usage endpoint returned 502 Bad Gateway: gateway temporarily unavailable")
    );
    assert!(after.last_action_at.is_some());
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);
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
    assert_eq!(summary.health_status, UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL);
    assert_eq!(summary.display_status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert_eq!(summary.work_status, UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED);
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
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED
    );
    assert_eq!(detail.summary.sync_state, UPSTREAM_ACCOUNT_SYNC_STATE_IDLE);
    assert_eq!(
        detail
            .recent_actions
            .first()
            .map(|event| event.action.as_str()),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED)
    );
    assert_eq!(
        detail
            .recent_actions
            .first()
            .and_then(|event| event.http_status),
        Some(502)
    );
    assert_eq!(usage_requests.load(Ordering::SeqCst), 2);
    assert_eq!(token_requests.load(Ordering::SeqCst), 1);
}

use super::*;
