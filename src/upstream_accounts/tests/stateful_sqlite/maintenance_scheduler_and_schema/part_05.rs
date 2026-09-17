use super::*;
use serde_json::json;

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
    let (state, account_ids) = prepare_compression_inheritance_test().await;
    let [root_only_id, group_only_id, account_override_id, oauth_id] = account_ids;

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

async fn prepare_compression_inheritance_test() -> (Arc<AppState>, [i64; 4]) {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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
    sqlx::query("UPDATE pool_upstream_account_group_notes SET policy_request_compression_algorithm = 'deflate', policy_codex_imagegen_rewrite_mode = 'fill_missing' WHERE group_name = 'compression-mixed'")
        .execute(&state.pool)
        .await
        .expect("save group compression override");
    sqlx::query("UPDATE pool_upstream_accounts SET policy_request_compression_algorithm = CASE id WHEN ?1 THEN 'zstd' WHEN ?2 THEN 'identity' ELSE policy_request_compression_algorithm END, policy_codex_imagegen_rewrite_mode = CASE id WHEN ?1 THEN 'force_remove' ELSE policy_codex_imagegen_rewrite_mode END WHERE id IN (?1, ?2)")
        .bind(account_override_id)
        .bind(oauth_id)
        .execute(&state.pool)
        .await
        .expect("save account compression overrides");
    (
        state,
        [root_only_id, group_only_id, account_override_id, oauth_id],
    )
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

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_uses_most_conservative_tag_fast_mode() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Fast Mode Merge").await;

    let mut fill_missing_rule = test_tag_routing_rule();
    fill_missing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
    let fill_missing_tag = insert_test_tag(&pool, "fast-fill", &fill_missing_rule)
        .await
        .expect("insert fill-missing tag");

    let mut force_remove_rule = test_tag_routing_rule();
    force_remove_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;
    let force_remove_tag = insert_test_tag(&pool, "fast-remove", &force_remove_rule)
        .await
        .expect("insert force-remove tag");

    sync_account_tag_links(
        &pool,
        account_id,
        &[fill_missing_tag.summary.id, force_remove_tag.summary.id],
    )
    .await
    .expect("attach fast-mode tags");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
}

#[tokio::test]
pub(crate) async fn list_tags_only_returns_system_tags_and_disables_writes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "System Tag Account").await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");
    let custom_tag_id = insert_legacy_custom_tag(
        &state.pool,
        "fast-mode-round-trip",
        &test_tag_routing_rule(),
    )
    .await;

    let Json(listed) = list_tags(
        State(state.clone()),
        Query(ListTagsQuery {
            search: None,
            has_accounts: None,
            allow_cut_in: None,
            allow_cut_out: None,
        }),
    )
    .await
    .expect("list tags");
    assert!(!listed.writes_enabled);
    assert!(
        listed
            .items
            .iter()
            .all(|item| item.system_key.as_deref().is_some())
    );
    assert!(
        listed
            .items
            .iter()
            .any(|item| item.system_key.as_deref() == Some("unsupported_model:gpt-5.5"))
    );
    assert!(listed.items.iter().all(|item| item.id != custom_tag_id));
}

#[tokio::test]
pub(crate) async fn sticky_key_preview_uses_the_final_attempt_request_compression() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Sticky compression preview").await;
    let occurred_at = format_utc_iso(Utc::now());
    let payload = json!({
        "stickyKey": "sticky-compression",
        "upstreamAccountId": account_id,
        "requestCompressionAlgorithm": "gzip",
    })
    .to_string();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (invoke_id, occurred_at, source, payload, raw_response)
        VALUES ('sticky-compression-invocation', ?1, 'proxy', ?2, '')
        "#,
    )
    .bind(&occurred_at)
    .bind(&payload)
    .execute(&state.pool)
    .await
    .expect("insert sticky invocation");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            attempt_public_id, invoke_id, occurred_at, endpoint, route_mode, sticky_key,
            upstream_account_id, attempt_index, distinct_account_index, same_account_retry_index,
            started_at, finished_at, status, phase, http_status,
            upstream_request_compression_algorithm, created_at
        )
        VALUES (
            'STICKYCOMP1', 'sticky-compression-invocation', ?1, '/v1/responses', 'pool',
            'sticky-compression', ?2, 1, 1, 0,
            ?1, ?1, 'success', 'completed', 200,
            'br', ?1
        ), (
            'STICKYCOMP2', 'sticky-compression-invocation', ?1, '/v1/responses', 'pool',
            'sticky-compression', ?2, 2, 1, 1,
            ?1, ?1, 'success', 'completed', 200,
            'zstd', ?1
        )
        "#,
    )
    .bind(&occurred_at)
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("insert sticky attempts");

    let rows = query_account_sticky_key_recent_invocations(
        &state.pool,
        account_id,
        &["sticky-compression".to_string()],
        5,
        None,
    )
    .await
    .expect("query sticky preview");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].request_compression_algorithm.as_deref(),
        Some("zstd")
    );

    upsert_sticky_route(&state.pool, "sticky-compression", account_id, &occurred_at)
        .await
        .expect("upsert sticky route");
    let response = build_account_sticky_keys_response(
        &state.pool,
        account_id,
        AccountStickyKeySelection::Count(5),
    )
    .await
    .expect("build sticky response");

    assert_eq!(
        response.conversations[0].recent_invocations[0]
            .request_compression_algorithm
            .as_deref(),
        Some("zstd")
    );
}

#[tokio::test]
pub(crate) async fn sticky_key_recent_preview_uses_compatible_composite_index_without_sql_sort() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Sticky preview index").await;
    assert_sticky_preview_index_and_plans(&state.pool, account_id).await;
    seed_and_assert_sticky_preview_rows(&state.pool, account_id).await;
}

async fn assert_sticky_preview_index_and_plans(pool: &SqlitePool, account_id: i64) {
    let index_sql = sqlx::query_scalar::<_, String>(
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'idx_codex_invocations_account_sticky_key_recent'",
    )
    .fetch_one(pool)
    .await
    .expect("load sticky preview index");
    for fragment in [
        "$.upstreamAccountId",
        "$.stickyKey",
        "$.promptCacheKey",
        "occurred_at DESC",
        "id DESC",
    ] {
        assert!(
            index_sql.contains(fragment),
            "missing index fragment: {fragment}"
        );
    }
    let keys = vec!["sticky-primary".to_string(), "sticky-fallback".to_string()];
    let plan = sticky_preview_explain_plan(pool, account_id, &keys, None).await;
    assert!(plan.contains("idx_codex_invocations_account_sticky_key_recent"));
    assert!(
        !plan.contains("USE TEMP B-TREE"),
        "sticky preview should not sort: {plan}"
    );
    let windowed_plan =
        sticky_preview_explain_plan(pool, account_id, &keys, Some("2026-08-20 10:00:00")).await;
    assert!(windowed_plan.contains("idx_codex_invocations_account_sticky_key_recent"));
    assert!(!windowed_plan.contains("USE TEMP B-TREE"));
}

async fn sticky_preview_explain_plan(
    pool: &SqlitePool,
    account_id: i64,
    keys: &[String],
    occurred_before: Option<&str>,
) -> String {
    let query =
        build_account_sticky_key_recent_invocations_query(account_id, keys, 5, occurred_before);
    let explain_sql = format!("EXPLAIN QUERY PLAN {}", query.sql());
    let mut request = sqlx::query(&explain_sql).bind(account_id);
    if let Some(occurred_before) = occurred_before {
        request = request.bind(occurred_before);
    }
    request
        .bind("sticky-primary")
        .bind("sticky-fallback")
        .bind(5_i64)
        .fetch_all(pool)
        .await
        .expect("load sticky preview explain plan")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ")
}

async fn seed_and_assert_sticky_preview_rows(pool: &SqlitePool, account_id: i64) {
    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, payload, raw_response) VALUES ('sticky-primary-old', '2026-08-20 10:00:00', 'proxy', ?1, ''), ('sticky-primary-tie-old', '2026-08-20 10:01:00', 'proxy', ?1, ''), ('sticky-primary-tie-new', '2026-08-20 10:01:00', 'proxy', ?1, ''), ('sticky-primary-new', '2026-08-20 10:02:00', 'proxy', ?1, ''), ('sticky-fallback-new', '2026-08-20 10:03:00', 'proxy', ?2, '')",
    )
    .bind(json!({"upstreamAccountId": account_id, "stickyKey": "sticky-primary"}).to_string())
    .bind(json!({"upstreamAccountId": account_id, "promptCacheKey": "sticky-fallback"}).to_string())
    .execute(pool)
    .await
    .expect("insert sticky preview invocations");
    let rows = query_account_sticky_key_recent_invocations(
        pool,
        account_id,
        &["sticky-primary".to_string(), "sticky-fallback".to_string()],
        3,
        None,
    )
    .await
    .expect("load indexed sticky preview");
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].sticky_key, "sticky-fallback");
    assert_eq!(rows[0].invoke_id, "sticky-fallback-new");
    assert_eq!(rows[1].sticky_key, "sticky-primary");
    assert_eq!(rows[1].invoke_id, "sticky-primary-new");
    assert_eq!(rows[2].invoke_id, "sticky-primary-tie-new");
    assert_eq!(rows[3].invoke_id, "sticky-primary-tie-old");
}

#[tokio::test]
#[ignore = "manual benchmark: measures indexed sticky previews on an isolated 300k fixture"]
pub(crate) async fn benchmark_sticky_key_recent_preview_on_three_hundred_thousand_rows() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("open sticky preview benchmark sqlite");
    create_sticky_benchmark_schema(&pool).await;
    seed_sticky_benchmark_rows(&pool).await;
    assert_sticky_benchmark_query(&pool).await;
}

async fn create_sticky_benchmark_schema(pool: &SqlitePool) {
    sqlx::query(
        "CREATE TABLE codex_invocations (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT NOT NULL, status TEXT, failure_class TEXT, failure_kind TEXT, error_message TEXT, model TEXT, total_tokens INTEGER, cost REAL, source TEXT, input_tokens INTEGER, output_tokens INTEGER, cache_input_tokens INTEGER, reasoning_tokens INTEGER, t_req_read_ms REAL, t_req_parse_ms REAL, t_upstream_connect_ms REAL, t_upstream_ttfb_ms REAL, first_token_ms REAL, t_upstream_stream_ms REAL, t_resp_parse_ms REAL, t_persist_ms REAL, t_total_ms REAL, payload TEXT)",
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark table");
    sqlx::query(
        "CREATE TABLE pool_upstream_request_attempts (id INTEGER PRIMARY KEY, invoke_id TEXT, occurred_at TEXT, status TEXT, upstream_request_compression_algorithm TEXT, attempt_index INTEGER)",
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark attempts table");
    sqlx::query(
        "CREATE INDEX idx_pool_upstream_request_attempts_invoke_attempt ON pool_upstream_request_attempts (invoke_id, attempt_index)",
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark attempts index");
}

async fn seed_sticky_benchmark_rows(pool: &SqlitePool) {
    let account_id = 42_i64;
    sqlx::query(
        r#"
        WITH RECURSIVE
            thousands(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM thousands WHERE value < 999
            ),
            hundreds(value) AS (
                VALUES(0)
                UNION ALL
                SELECT value + 1 FROM hundreds WHERE value < 299
            )
        INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, payload)
        SELECT
            thousands.value * 300 + hundreds.value + 1,
            printf('sticky-benchmark-%06d', thousands.value * 300 + hundreds.value + 1),
            datetime('2026-08-20 00:00:00', printf('+%d seconds', thousands.value * 300 + hundreds.value)),
            'proxy',
            json_object(
                'upstreamAccountId', CASE WHEN (thousands.value * 300 + hundreds.value) % 10 = 0 THEN ?1 ELSE ?1 + 1 END,
                CASE WHEN (thousands.value * 300 + hundreds.value) % 20 = 0 THEN 'stickyKey' ELSE 'promptCacheKey' END,
                printf('sticky-%02d', (thousands.value * 300 + hundreds.value) % 12)
            )
        FROM thousands
        CROSS JOIN hundreds
        "#,
    )
    .bind(account_id)
    .execute(pool)
    .await
    .expect("seed 300k sticky preview fixture");
    sqlx::query(
        r#"
        CREATE INDEX idx_codex_invocations_account_sticky_key_recent
        ON codex_invocations (
            (CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END),
            (CASE WHEN json_valid(payload) THEN TRIM(COALESCE(CAST(json_extract(payload, '$.stickyKey') AS TEXT), CAST(json_extract(payload, '$.promptCacheKey') AS TEXT))) END),
            occurred_at DESC,
            id DESC
        )
        "#,
    )
    .execute(pool)
    .await
    .expect("create sticky preview benchmark index");
}

async fn assert_sticky_benchmark_query(pool: &SqlitePool) {
    let account_id = 42_i64;
    let selected_keys = vec![
        "sticky-00".to_string(),
        "sticky-02".to_string(),
        "sticky-04".to_string(),
    ];
    let explain_query =
        build_account_sticky_key_recent_invocations_query(account_id, &selected_keys, 5, None);
    let explain_sql = explain_query.sql().to_string();
    let plan = sqlx::query(&format!("EXPLAIN QUERY PLAN {explain_sql}"))
        .bind(account_id)
        .bind("sticky-00")
        .bind("sticky-02")
        .bind("sticky-04")
        .bind(5_i64)
        .fetch_all(pool)
        .await
        .expect("load 300k sticky preview explain plan")
        .into_iter()
        .map(|row| row.get::<String, _>("detail"))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        plan.contains("idx_codex_invocations_account_sticky_key_recent"),
        "unexpected 300k sticky preview plan: {plan}"
    );
    assert!(
        !plan.contains("USE TEMP B-TREE"),
        "300k sticky preview should not sort in SQLite: {plan}"
    );

    let rows =
        query_account_sticky_key_recent_invocations(pool, account_id, &selected_keys, 5, None)
            .await
            .expect("warm indexed sticky preview");
    assert_eq!(rows.len(), 15);

    let started_at = std::time::Instant::now();
    let rows =
        query_account_sticky_key_recent_invocations(pool, account_id, &selected_keys, 5, None)
            .await
            .expect("load indexed sticky preview");
    let elapsed = started_at.elapsed();

    assert_eq!(rows.len(), 15);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "indexed sticky preview exceeded one second on the 300k-row fixture: {elapsed:?}"
    );
    eprintln!("indexed 300k sticky preview completed in {elapsed:?}");
}
