use super::*;

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
                codex_imagegen_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                oauth_request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Missing,
                available_models_mode: OptionalField::Missing,
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
                codex_imagegen_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                oauth_request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Null,
                available_models_mode: OptionalField::Missing,
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
async fn update_upstream_account_group_preserves_available_models_when_mode_changes() {
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
                codex_imagegen_rewrite_mode: OptionalField::Missing,
                request_compression_algorithm: OptionalField::Missing,
                oauth_request_compression_algorithm: OptionalField::Missing,
                concurrency_limit: OptionalField::Missing,
                upstream_429_retry_enabled: OptionalField::Missing,
                upstream_429_max_retries: OptionalField::Missing,
                available_models: OptionalField::Missing,
                available_models_mode: OptionalField::Value("denylist".to_string()),
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

    let stored_mode = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT policy_available_models_mode
            FROM pool_upstream_account_group_notes
            WHERE group_name = 'preserve-model-group'
            "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load preserved group policy mode");
    assert_eq!(stored_mode.as_deref(), Some("denylist"));
}

#[tokio::test]
async fn create_api_key_account_rejects_group_fields() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Rejected Group Account",
        "apiKey": "sk-rejected-group",
        "groupName": "legacy-group"
    }))
    .expect("deserialize api key create request");
    let err = create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect_err("API key group fields must be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pool_upstream_accounts WHERE display_name = 'Rejected Group Account'",
        )
        .fetch_one(&state.pool)
        .await
        .expect("count rejected account"),
        0
    );
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

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("cannot use groups"));
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

    let err = update_upstream_account(
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
    .expect_err("API key group policy updates must be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    let metadata = load_group_metadata(&state.pool, Some("shared-update-group"))
        .await
        .expect("load shared update group metadata");
    assert!(!metadata.node_shunt_enabled);
}

#[tokio::test]
async fn api_key_group_migration_is_idempotent_and_preserves_oauth_groups() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let api_key_id = insert_api_key_account(&state.pool, "Legacy Relay Group").await;
    set_test_account_group_name(&state.pool, api_key_id, Some("legacy-relay-group")).await;
    sqlx::query("UPDATE pool_upstream_accounts SET is_mother = 1 WHERE id = ?1")
        .bind(api_key_id)
        .execute(&state.pool)
        .await
        .expect("mark legacy API key mother");
    let oauth_id = insert_oauth_account(&state.pool, "Official OAuth Member").await;
    set_test_account_group_name(&state.pool, oauth_id, Some("legacy-relay-group")).await;

    let Json(preflight) = preflight_api_key_group_migration(State(state.clone()))
        .await
        .expect("preflight migration");
    assert_eq!(preflight.api_key_count, 1);
    assert!(preflight.blocked_strategies.is_empty());
    assert!(preflight.can_migrate);

    let Json(migrated) = confirm_api_key_group_migration(
        State(state.clone()),
        HeaderMap::new(),
        Json(ConfirmApiKeyGroupMigrationRequest {
            confirmation_hash: "stale".to_string(),
            disabled_strategies: Vec::new(),
        }),
    )
    .await
    .expect("migration automatically disables obsolete group strategies");
    assert_eq!(migrated.migrated_count, 1);
    let api_key_state = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT group_name, is_mother FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(api_key_id)
    .fetch_one(&state.pool)
    .await
    .expect("load migrated API key");
    assert_eq!(api_key_state, (None, 0));
    let oauth_group = sqlx::query_scalar::<_, Option<String>>(
        "SELECT group_name FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(oauth_id)
    .fetch_one(&state.pool)
    .await
    .expect("load OAuth group");
    assert_eq!(oauth_group.as_deref(), Some("legacy-relay-group"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM pool_upstream_account_events WHERE account_id = ?1 AND action = 'api_key_transit_proxy_binding_migrated'",
        )
        .bind(api_key_id)
        .fetch_one(&state.pool)
        .await
        .expect("count migration audit"),
        1
    );
    let Json(post_migration_preflight) = preflight_api_key_group_migration(State(state.clone()))
        .await
        .expect("preflight after migration");
    assert_eq!(post_migration_preflight.api_key_count, 0);
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
        reserve: None,
        ..Default::default()
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
        reserve: None,
        ..Default::default()
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
            runtime_routing_generation: 0,
            id: 1,
            kind: kind.to_string(),
            provider: UPSTREAM_ACCOUNT_PROVIDER_CODEX.to_string(),
            display_name: "Test".to_string(),
            group_name: None,
            bound_proxy_keys_json: None,
            model_mappings_json: None,
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
            access_token_expiry_source: None,
            access_token_expiry_observed_at: None,
            refresh_token_expires_at: None,
            refresh_token_expiry_source: None,
            refresh_token_expiry_state: None,
            refresh_token_expiry_observed_at: None,
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
            chat_completions_capability: None,
            chat_completions_capability_observed_at: None,
            chat_completions_capability_reason: None,
            policy_chat_completions_capability_override: None,
            image_endpoint_capability: None,
            image_endpoint_capability_observed_at: None,
            image_endpoint_capability_reason: None,
            policy_image_endpoint_capability_override: None,
            response_image_tool_capability: None,
            response_image_tool_capability_observed_at: None,
            response_image_tool_capability_reason: None,
            policy_response_image_tool_capability_override: None,
            codex_imagegen_capability: None,
            codex_imagegen_capability_observed_at: None,
            codex_imagegen_capability_reason: None,
            policy_codex_imagegen_capability_override: None,
            standalone_search_capability: None,
            standalone_search_capability_observed_at: None,
            standalone_search_capability_reason: None,
            policy_standalone_search_capability_override: None,
            local_primary_limit: None,
            local_secondary_limit: None,
            local_limit_unit: None,
            policy_allow_cut_out: None,
            policy_allow_cut_in: None,
            policy_priority_tier: None,
            policy_fast_mode_rewrite_mode: None,
            policy_image_tool_rewrite_mode: None,
            policy_codex_imagegen_rewrite_mode: None,
            policy_request_compression_algorithm: None,
            policy_oauth_request_compression_algorithm: None,
            policy_concurrency_limit: None,
            policy_upstream_429_retry_enabled: None,
            policy_upstream_429_max_retries: None,
            policy_available_models_json: None,
            policy_available_models_mode: None,
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
