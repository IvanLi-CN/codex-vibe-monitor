#[tokio::test]
pub(crate) async fn update_upstream_account_patches_one_timeout_without_clearing_other_overrides() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Patch Timeout Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_responses_first_byte_timeout_secs = 180,
                policy_compact_first_byte_timeout_secs = 300,
                policy_image_first_byte_timeout_secs = 360,
                policy_responses_stream_timeout_secs = 1800,
                policy_compact_stream_timeout_secs = 300
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account timeout overrides");
    assert_eq!(
        load_upstream_account_row(&state.pool, account_id)
            .await
            .expect("load seeded account")
            .expect("seeded account exists")
            .policy_image_first_byte_timeout_secs,
        Some(360)
    );

    let detail = patch_responses_stream_timeout(&state, account_id).await;

    let stored_image_timeout: Option<i64> = sqlx::query_scalar(
        "SELECT policy_image_first_byte_timeout_secs FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load stored image timeout override");
    assert_eq!(stored_image_timeout, Some(360));

    let response_rule = detail.summary.effective_routing_rule;
    assert_eq!(
        response_rule.timeouts.responses_first_byte_timeout_secs,
        Some(180)
    );
    assert_eq!(
        response_rule.timeouts.compact_first_byte_timeout_secs,
        Some(300)
    );
    assert_eq!(
        response_rule.timeouts.image_first_byte_timeout_secs,
        Some(360)
    );
    assert_eq!(
        response_rule.timeouts.responses_stream_timeout_secs,
        Some(1900)
    );
    assert_eq!(
        response_rule.timeouts.compact_stream_timeout_secs,
        Some(300)
    );
    assert_eq!(
        response_rule
            .timeout_field_sources
            .responses_first_byte_timeout_secs,
        "account"
    );
    assert_eq!(
        response_rule
            .timeout_field_sources
            .responses_stream_timeout_secs,
        "account"
    );

    let stored = sqlx::query_as::<_, (Option<i64>, Option<i64>, Option<i64>, Option<i64>, Option<i64>)>(
            "SELECT policy_responses_first_byte_timeout_secs, policy_compact_first_byte_timeout_secs, policy_image_first_byte_timeout_secs, policy_responses_stream_timeout_secs, policy_compact_stream_timeout_secs FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored timeout policy");
    assert_eq!(
        stored,
        (Some(180), Some(300), Some(360), Some(1900), Some(300))
    );

    assert_reloaded_timeout_policy(&state.pool, account_id).await;
}

async fn patch_responses_stream_timeout(
    state: &Arc<AppState>,
    account_id: i64,
) -> UpstreamAccountDetail {
    state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: Some(UpdateRoutingTimeoutSettingsRequest {
                        responses_stream_timeout_secs: OptionalField::Value(1900),
                        ..UpdateRoutingTimeoutSettingsRequest::default()
                    }),
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("patch one account timeout field")
}

async fn assert_reloaded_timeout_policy(pool: &SqlitePool, account_id: i64) {
    let reloaded_rule = load_effective_routing_rule_for_account(pool, account_id)
        .await
        .expect("reload effective routing rule");
    assert_eq!(
        reloaded_rule.timeouts.responses_first_byte_timeout_secs,
        Some(180)
    );
    assert_eq!(
        reloaded_rule.timeouts.compact_first_byte_timeout_secs,
        Some(300)
    );
    assert_eq!(
        reloaded_rule.timeouts.image_first_byte_timeout_secs,
        Some(360)
    );
    assert_eq!(
        reloaded_rule.timeouts.responses_stream_timeout_secs,
        Some(1900)
    );
    assert_eq!(
        reloaded_rule.timeouts.compact_stream_timeout_secs,
        Some(300)
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_writes_positive_new_conversation_policy() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Positive Account Policy").await;

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
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Value("no_new".to_string()),
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save positive new conversation policy");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        "SELECT policy_priority_tier FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load stored policy");
    assert_eq!(stored, Some("no_new".to_string()));

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load effective routing rule");
    assert_eq!(rule.priority_tier, TagPriorityTier::NoNew);
    assert_eq!(rule.field_sources.priority_tier, "account");
}

#[tokio::test]
pub(crate) async fn update_upstream_account_preserves_priority_tier_when_omitted() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Preserve Legacy Block").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_priority_tier = 'no_new'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed positive and legacy new conversation policy");

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
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Value(false),
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save unrelated account policy field");

    let stored = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
            "SELECT policy_priority_tier, policy_allow_cut_out FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored policy");
    assert_eq!(stored, (Some("no_new".to_string()), Some(0)));
}

#[tokio::test]
pub(crate) async fn update_upstream_account_accepts_no_new_priority_write() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Legacy Block Write").await;

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
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Value("no_new".to_string()),
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save legacy block new conversations policy");

    let stored = sqlx::query_scalar::<_, Option<String>>(
        "SELECT policy_priority_tier FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load stored policy");
    assert_eq!(stored, Some("no_new".to_string()));

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load effective routing rule");
    assert_eq!(rule.priority_tier, TagPriorityTier::NoNew);
    assert_eq!(rule.field_sources.priority_tier, "account");
}

#[tokio::test]
pub(crate) async fn update_upstream_account_does_not_change_priority_tier_when_omitted() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Legacy Only Missing").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_priority_tier = 'no_new'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed legacy-only new conversation policy");

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
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Value(false),
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save unrelated account policy field");

    let stored = sqlx::query_as::<_, (Option<String>, Option<i64>)>(
            "SELECT policy_priority_tier, policy_allow_cut_out FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored policy");
    assert_eq!(stored, (Some("no_new".to_string()), Some(0)));
}

#[tokio::test]
pub(crate) async fn update_upstream_account_persists_empty_available_models_as_deny_all() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Deny All Models").await;

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
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Missing,
                    fast_mode_rewrite_mode: OptionalField::Missing,
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Value(vec![]),
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save empty model override");

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load effective routing rule");
    assert!(rule.available_models_defined);
    assert!(rule.available_models.is_empty());
    assert_eq!(rule.field_sources.available_models, "account");
}

#[tokio::test]
pub(crate) async fn update_upstream_account_rejects_invalid_routing_policy_enums() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Invalid Account Policy").await;

    let err = state
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
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Missing,
                    priority_tier: OptionalField::Value("normal".to_string()),
                    fast_mode_rewrite_mode: OptionalField::Value("always_fast".to_string()),
                    image_tool_rewrite_mode: OptionalField::Missing,
                    codex_imagegen_rewrite_mode: OptionalField::Missing,
                    request_compression_algorithm: OptionalField::Missing,
                    concurrency_limit: OptionalField::Missing,
                    upstream_429_retry_enabled: OptionalField::Missing,
                    upstream_429_max_retries: OptionalField::Missing,
                    available_models: OptionalField::Missing,
                    available_models_mode: OptionalField::Missing,
                    status_change_reasons: None,
                    timeouts: None,
                }),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect_err("invalid routing policy enum should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
    );
}

#[tokio::test]
pub(crate) async fn load_upstream_account_detail_with_actual_usage_returns_layered_effective_policy()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Layered Detail").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                policy_fast_mode_rewrite_mode = 'force_remove'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("detail-layered")
    .execute(&state.pool)
    .await
    .expect("assign account detail policy");

    let mut tag_rule = test_tag_routing_rule();
    tag_rule.allow_cut_in = false;
    tag_rule.priority_tier = TagPriorityTier::Fallback;
    tag_rule.upstream_429_retry_enabled = true;
    tag_rule.upstream_429_max_retries = 2;
    let tag = insert_test_tag(&state.pool, "detail-layered-tag", &tag_rule)
        .await
        .expect("insert detail tag");
    sync_account_tag_links(&state.pool, account_id, &[tag.summary.id])
        .await
        .expect("attach detail tag");

    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "detail-layered",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 7,
        },
    )
    .await
    .expect("save detail group metadata");
    drop(conn);
    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_priority_tier = 'primary',
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_concurrency_limit = 5
            WHERE group_name = 'detail-layered'
            "#,
    )
    .execute(&state.pool)
    .await
    .expect("save detail group policy");

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .expect("load account detail")
        .expect("detail exists");
    let rule = detail.summary.effective_routing_rule;
    assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
    assert_eq!(rule.field_sources.priority_tier, "tag");
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
    assert_eq!(rule.field_sources.fast_mode_rewrite_mode, "account");
    assert_eq!(rule.concurrency_limit, 0);
    assert_eq!(rule.field_sources.concurrency_limit, "tag");
    assert!(!rule.allow_cut_in);
    assert_eq!(rule.field_sources.allow_cut_in, "tag");
    assert!(rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 2);
    assert_eq!(rule.field_sources.upstream_429_retry, "tag");
}

#[tokio::test]
pub(crate) async fn load_upstream_account_detail_with_actual_usage_populates_root_timeout_values() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Root Timeout Detail").await;
    sqlx::query(
        r#"
            UPDATE pool_routing_settings
            SET responses_first_byte_timeout_secs = 41,
                compact_first_byte_timeout_secs = 42,
                image_first_byte_timeout_secs = 45,
                responses_stream_timeout_secs = 43,
                compact_stream_timeout_secs = 44
            WHERE id = ?1
            "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(&state.pool)
    .await
    .expect("save root timeout settings");

    let detail = load_upstream_account_detail_with_actual_usage(state.as_ref(), account_id)
        .await
        .expect("load account detail")
        .expect("detail exists");
    let rule = detail.summary.effective_routing_rule;

    assert_eq!(rule.timeouts.responses_first_byte_timeout_secs, Some(41));
    assert_eq!(rule.timeouts.compact_first_byte_timeout_secs, Some(42));
    assert_eq!(rule.timeouts.image_first_byte_timeout_secs, Some(45));
    assert_eq!(rule.timeouts.responses_stream_timeout_secs, Some(43));
    assert_eq!(rule.timeouts.compact_stream_timeout_secs, Some(44));
    assert_eq!(
        rule.timeout_field_sources.responses_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.compact_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.image_first_byte_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.responses_stream_timeout_secs,
        "root"
    );
    assert_eq!(
        rule.timeout_field_sources.compact_stream_timeout_secs,
        "root"
    );
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rules_for_accounts_request_compression_respects_inheritance()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (root_only_id, group_only_id, account_override_id, oauth_id) =
        seed_request_compression_inheritance(&state).await;

    let rules = load_effective_routing_rules_for_accounts(
        &state.pool,
        &[root_only_id, group_only_id, account_override_id, oauth_id],
    )
    .await
    .expect("load effective routing rules");

    let root_only = rules.get(&root_only_id).expect("root-only rule");
    assert_eq!(
        root_only.request_compression_algorithm,
        RequestCompressionAlgorithm::Gzip
    );
    assert_eq!(
        root_only.field_sources.request_compression_algorithm,
        "root"
    );
    assert_eq!(
        root_only.codex_imagegen_rewrite_mode,
        CodexImagegenRewriteMode::ForceAdd
    );
    assert_eq!(root_only.field_sources.codex_imagegen_rewrite_mode, "root");

    let group_only = rules.get(&group_only_id).expect("group-only rule");
    assert_eq!(
        group_only.request_compression_algorithm,
        RequestCompressionAlgorithm::Gzip
    );
    assert_eq!(
        group_only.field_sources.request_compression_algorithm,
        "root"
    );
    assert_eq!(
        group_only.codex_imagegen_rewrite_mode,
        CodexImagegenRewriteMode::ForceAdd
    );
    assert_eq!(group_only.field_sources.codex_imagegen_rewrite_mode, "root");

    let account_override = rules
        .get(&account_override_id)
        .expect("account override rule");
    assert_eq!(
        account_override.request_compression_algorithm,
        RequestCompressionAlgorithm::Zstd
    );
    assert_eq!(
        account_override.field_sources.request_compression_algorithm,
        "account"
    );
    assert_eq!(
        account_override.codex_imagegen_rewrite_mode,
        CodexImagegenRewriteMode::ForceRemove
    );
    assert_eq!(
        account_override.field_sources.codex_imagegen_rewrite_mode,
        "account"
    );

    let oauth_rule = rules.get(&oauth_id).expect("oauth rule");
    assert_eq!(
        oauth_rule.request_compression_algorithm,
        RequestCompressionAlgorithm::Gzip
    );
    assert_eq!(
        oauth_rule.field_sources.request_compression_algorithm,
        "root"
    );
    assert_eq!(
        oauth_rule.codex_imagegen_rewrite_mode,
        CodexImagegenRewriteMode::FillMissing
    );
    assert_eq!(
        oauth_rule.field_sources.codex_imagegen_rewrite_mode,
        "group"
    );
}

async fn seed_request_compression_inheritance(state: &AppState) -> (i64, i64, i64, i64) {
    let root_only_id = insert_api_key_account(&state.pool, "Root Compression").await;
    let group_only_id = insert_api_key_account(&state.pool, "Group Compression").await;
    let account_override_id = insert_api_key_account(&state.pool, "Account Compression").await;
    let oauth_id = insert_oauth_account(&state.pool, "OAuth Compression").await;
    save_pool_routing_settings(
        &state.pool,
        &state.config,
        PoolRoutingSettingsUpdate {
            crypto_key: None,
            api_key: None,
            request_compression_algorithm: Some(RequestCompressionAlgorithm::Gzip),
            request_compression_level_preset: Some(RequestCompressionLevelPreset::Best),
            codex_imagegen_rewrite_mode: Some(CodexImagegenRewriteMode::ForceAdd),
            available_models: None,
            available_models_mode: None,
            timeout_updates: None,
            maintenance_settings: None,
            cache_hit_protection: None,
            priority_handoff_admission_enabled: None,
        },
    )
    .await
    .expect("save root request compression settings");
    for account_id in [group_only_id, account_override_id, oauth_id] {
        sqlx::query(
            "UPDATE pool_upstream_accounts SET group_name = 'compression-mixed' WHERE id = ?1",
        )
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("assign mixed compression group");
    }
    let mut conn = state.pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "compression-mixed",
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
    .expect("save compression group metadata");
    drop(conn);
    sqlx::query(
        "UPDATE pool_upstream_account_group_notes SET policy_request_compression_algorithm = 'deflate', policy_codex_imagegen_rewrite_mode = 'fill_missing' WHERE group_name = 'compression-mixed'",
    )
    .execute(&state.pool)
    .await
    .expect("save group compression override");
    sqlx::query(
        "UPDATE pool_upstream_accounts SET policy_request_compression_algorithm = CASE id WHEN ?1 THEN 'zstd' WHEN ?2 THEN 'identity' ELSE policy_request_compression_algorithm END, policy_codex_imagegen_rewrite_mode = CASE id WHEN ?1 THEN 'force_remove' ELSE policy_codex_imagegen_rewrite_mode END WHERE id IN (?1, ?2)",
    )
    .bind(account_override_id)
    .bind(oauth_id)
    .execute(&state.pool)
    .await
    .expect("save account compression overrides");
    (root_only_id, group_only_id, account_override_id, oauth_id)
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_uses_most_conservative_tag_priority() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Priority Merge").await;

    let mut primary_rule = test_tag_routing_rule();
    primary_rule.priority_tier = TagPriorityTier::Primary;
    let primary_tag = insert_test_tag(&pool, "priority-primary", &primary_rule)
        .await
        .expect("insert primary tag");

    let mut fallback_rule = test_tag_routing_rule();
    fallback_rule.priority_tier = TagPriorityTier::Fallback;
    let fallback_tag = insert_test_tag(&pool, "priority-fallback", &fallback_rule)
        .await
        .expect("insert fallback tag");

    sync_account_tag_links(
        &pool,
        account_id,
        &[primary_tag.summary.id, fallback_tag.summary.id],
    )
    .await
    .expect("attach priority tags");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
    let mut source_tag_ids = rule.source_tag_ids.clone();
    source_tag_ids.sort_unstable();
    let mut expected_tag_ids = vec![primary_tag.summary.id, fallback_tag.summary.id];
    expected_tag_ids.sort_unstable();
    assert_eq!(source_tag_ids, expected_tag_ids);
}

use super::*;
