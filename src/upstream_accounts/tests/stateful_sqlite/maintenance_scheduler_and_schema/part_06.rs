#[test]
pub(crate) fn build_effective_routing_rule_uses_most_conservative_priority_tier() {
    let mut primary = test_account_tag_summary(1, "primary", 0);
    primary.routing_rule.priority_tier = TagPriorityTier::Primary;
    let mut normal = test_account_tag_summary(2, "normal", 0);
    normal.routing_rule.priority_tier = TagPriorityTier::Normal;
    let mut fallback = test_account_tag_summary(3, "fallback", 0);
    fallback.routing_rule.priority_tier = TagPriorityTier::Fallback;

    let rule = build_effective_routing_rule(&[primary, normal, fallback]);

    assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
}

#[test]
pub(crate) fn normalize_tag_fast_mode_rewrite_mode_defaults_to_keep_original_and_rejects_invalid_values()
 {
    assert_eq!(
        normalize_tag_fast_mode_rewrite_mode(None),
        Ok(TagFastModeRewriteMode::KeepOriginal)
    );
    assert_eq!(
        normalize_tag_fast_mode_rewrite_mode(Some("force_add")),
        Ok(TagFastModeRewriteMode::ForceAdd)
    );
    assert_eq!(
            normalize_tag_fast_mode_rewrite_mode(Some("unexpected")),
            Err((
                StatusCode::BAD_REQUEST,
                "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add".to_string(),
            ))
        );
}

#[test]
pub(crate) fn build_effective_routing_rule_uses_most_conservative_fast_mode_rewrite_mode() {
    let mut keep_original = test_account_tag_summary(1, "keep", 0);
    keep_original.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::KeepOriginal;
    let mut fill_missing = test_account_tag_summary(2, "fill", 0);
    fill_missing.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
    let mut force_add = test_account_tag_summary(3, "add", 0);
    force_add.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceAdd;
    let mut force_remove = test_account_tag_summary(4, "remove", 0);
    force_remove.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;

    let rule =
        build_effective_routing_rule(&[keep_original, fill_missing, force_add, force_remove]);

    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
}

#[test]
pub(crate) fn build_effective_routing_rule_intersects_available_models_and_collects_system_denies()
{
    let mut first = test_account_tag_summary(1, "first", 0);
    first.routing_rule.available_models = vec!["gpt-5.5".to_string(), "gpt-5.4-mini".to_string()];
    let mut second = test_account_tag_summary(2, "second", 0);
    second.routing_rule.available_models = vec!["gpt-5.4-mini".to_string(), "gpt-4.1".to_string()];
    second.system_key = Some("unsupported_model:gpt-5.5".to_string());

    let rule = build_effective_routing_rule(&[first, second]);

    assert_eq!(rule.available_models, vec!["gpt-5.4-mini".to_string()]);
    assert!(rule.available_models_defined);
    assert_eq!(rule.field_sources.available_models, "tag");
    assert_eq!(rule.system_denied_models, vec!["gpt-5.5".to_string()]);
    assert_eq!(rule.field_sources.system_denied_models, "system");
}

#[test]
pub(crate) fn build_effective_routing_rule_intersects_available_models_by_alias() {
    let mut first = test_account_tag_summary(1, "first", 0);
    first.routing_rule.available_models = vec!["gpt-5.5-2026-01-15".to_string()];
    let mut second = test_account_tag_summary(2, "second", 0);
    second.routing_rule.available_models = vec!["gpt-5.5".to_string()];

    let rule = build_effective_routing_rule(&[first, second]);

    assert_eq!(
        rule.available_models,
        vec!["gpt-5.5-2026-01-15".to_string()]
    );
    assert!(rule.available_models_defined);
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &rule));
}

#[test]
pub(crate) fn build_effective_routing_rule_keeps_disjoint_tag_model_intersection_as_deny_all() {
    let mut first = test_account_tag_summary(1, "first", 0);
    first.routing_rule.available_models = vec!["gpt-4o".to_string()];
    let mut second = test_account_tag_summary(2, "second", 0);
    second.routing_rule.available_models = vec!["o3".to_string()];

    let rule = build_effective_routing_rule(&[first, second]);

    assert!(rule.available_models_defined);
    assert!(rule.available_models.is_empty());
    assert!(!account_accepts_requested_model(Some("gpt-4o"), &rule));
    assert!(!account_accepts_requested_model(Some("o3"), &rule));
}

#[test]
pub(crate) fn build_effective_routing_rule_ignores_editable_fields_from_protected_system_tags() {
    let mut system_tag = test_account_tag_summary(1, "system", 3);
    system_tag.protected = true;
    system_tag.system_key = Some("unsupported_model:gpt-5.4".to_string());
    system_tag.routing_rule.allow_cut_in = false;
    system_tag.routing_rule.priority_tier = TagPriorityTier::Fallback;
    system_tag.routing_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::ForceRemove;
    system_tag.routing_rule.upstream_429_retry_enabled = true;
    system_tag.routing_rule.upstream_429_max_retries = 4;
    system_tag.routing_rule.available_models = vec!["gpt-5.4-mini".to_string()];

    let rule = build_effective_routing_rule(&[system_tag]);

    assert!(rule.allow_cut_in);
    assert_eq!(rule.priority_tier, TagPriorityTier::Normal);
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::KeepOriginal
    );
    assert_eq!(rule.concurrency_limit, 0);
    assert!(!rule.upstream_429_retry_enabled);
    assert_eq!(rule.available_models, vec!["gpt-5.4-mini".to_string()]);
    assert_eq!(rule.system_denied_models, vec!["gpt-5.4".to_string()]);
}

#[test]
pub(crate) fn root_and_lower_model_policies_fail_closed_on_blank_entries() {
    let mut root_rule = test_effective_routing_rule(0);
    apply_root_available_models(&mut root_rule, Some(r#"[" "]"#), Some("denylist"));
    assert!(root_rule.available_models_defined);
    assert!(root_rule.available_models.is_empty());
    assert_eq!(
        root_rule.available_models_mode,
        AvailableModelsMode::Allowlist
    );
    assert!(!account_accepts_requested_model(
        Some("gpt-5.4"),
        &root_rule
    ));

    let mut lower_rule = test_effective_routing_rule(0);
    apply_routing_policy_override(
        &mut lower_rule,
        RoutingPolicyOverride {
            source: "group",
            allow_cut_out: None,
            allow_cut_in: None,
            priority_tier: None,
            fast_mode_rewrite_mode: None,
            image_tool_rewrite_mode: None,
            codex_imagegen_rewrite_mode: None,
            request_compression_algorithm: None,
            allow_request_compression_override: false,
            concurrency_limit: None,
            upstream_429_retry_enabled: None,
            upstream_429_max_retries: None,
            available_models_json: Some(r#"[" "]"#),
            available_models_mode: Some("denylist"),
        },
    );
    assert!(lower_rule.available_models_defined);
    assert!(lower_rule.available_models.is_empty());
    assert_eq!(
        lower_rule.available_models_mode,
        AvailableModelsMode::Allowlist
    );
    assert!(!account_accepts_requested_model(
        Some("gpt-5.4"),
        &lower_rule
    ));
}

#[test]
pub(crate) fn apply_tag_layer_routing_policy_preserves_inherited_available_models_when_tags_do_not_define_them()
 {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-5.5".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let tag_rule = build_effective_routing_rule(&[test_account_tag_summary(1, "tag", 0)]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(inherited.available_models, vec!["gpt-5.5".to_string()]);
    assert!(inherited.available_models_defined);
    assert_eq!(inherited.field_sources.available_models, "group");
}

#[test]
pub(crate) fn apply_tag_layer_routing_policy_intersects_tag_models_with_inherited_group_models() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-4o".to_string(), "gpt-5.5".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.5".to_string(), "o3".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(inherited.available_models, vec!["gpt-5.5".to_string()]);
    assert!(inherited.available_models_defined);
    assert_eq!(inherited.field_sources.available_models, "tag");
}

#[test]
pub(crate) fn apply_tag_layer_routing_policy_intersects_inherited_models_by_alias() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-5.5-2026-01-15".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.5".to_string(), "o3".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(
        inherited.available_models,
        vec!["gpt-5.5-2026-01-15".to_string()]
    );
    assert!(inherited.available_models_defined);
    assert_eq!(inherited.field_sources.available_models, "tag");
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &inherited));
}

#[test]
pub(crate) fn apply_tag_layer_routing_policy_keeps_group_tag_disjoint_models_as_deny_all() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-4o".to_string()];
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.5".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert!(inherited.available_models_defined);
    assert!(inherited.available_models.is_empty());
    assert!(!account_accepts_requested_model(Some("gpt-4o"), &inherited));
    assert!(!account_accepts_requested_model(
        Some("gpt-5.5"),
        &inherited
    ));
    assert_eq!(inherited.field_sources.available_models, "tag");
}

#[test]
pub(crate) fn apply_tag_layer_routing_policy_keeps_inherited_denylist_and_adds_tag_constraint() {
    let mut inherited = test_effective_routing_rule(0);
    inherited.available_models = vec!["gpt-5.4".to_string()];
    inherited.available_models_mode = AvailableModelsMode::Denylist;
    inherited.available_models_defined = true;
    inherited.field_sources.available_models = "group".to_string();
    inherited.field_sources.available_models_mode = "group".to_string();

    let mut tag = test_account_tag_summary(1, "tag", 0);
    tag.routing_rule.available_models = vec!["gpt-5.4".to_string(), "gpt-4.1".to_string()];
    let tag_rule = build_effective_routing_rule(&[tag]);

    apply_tag_layer_routing_policy(&mut inherited, &tag_rule);

    assert_eq!(
        inherited.available_models_mode,
        AvailableModelsMode::Denylist
    );
    assert_eq!(inherited.available_models, vec!["gpt-5.4".to_string()]);
    assert_eq!(
        inherited.tag_available_models,
        Some(vec!["gpt-5.4".to_string(), "gpt-4.1".to_string()])
    );
    assert!(!account_accepts_requested_model(
        Some("gpt-5.4"),
        &inherited
    ));
    assert!(account_accepts_requested_model(Some("gpt-4.1"), &inherited));
}

#[test]
pub(crate) fn malformed_available_models_mode_keeps_legacy_allowlist_semantics() {
    assert_eq!(
        AvailableModelsMode::from_str(Some("unexpected")),
        AvailableModelsMode::Allowlist
    );
}

#[test]
pub(crate) fn account_accepts_requested_model_supports_exact_alias_and_system_deny() {
    let mut rule = test_effective_routing_rule(0);
    rule.available_models = vec!["gpt-5.5-2026-01-15".to_string()];
    rule.available_models_defined = true;
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &rule));
    assert!(account_accepts_requested_model(
        Some("gpt-5.5-2026-01-15"),
        &rule
    ));
    assert!(!account_accepts_requested_model(Some("gpt-4.1"), &rule));

    rule.system_denied_models = vec!["gpt-5.5".to_string()];
    assert!(!account_accepts_requested_model(
        Some("gpt-5.5-2026-01-15"),
        &rule
    ));
    assert!(account_accepts_requested_model(None, &rule));
}

#[test]
pub(crate) fn account_accepts_concurrency_limit_treats_zero_as_unlimited_and_allows_sticky_reuse() {
    let unlimited = test_effective_routing_rule(0);
    let limited = test_effective_routing_rule(2);

    assert!(account_accepts_concurrency_limit(
        99,
        PoolRoutingSelectionSource::FreshAssignment,
        &unlimited,
    ));
    assert!(account_accepts_concurrency_limit(
        1,
        PoolRoutingSelectionSource::FreshAssignment,
        &limited,
    ));
    assert!(!account_accepts_concurrency_limit(
        2,
        PoolRoutingSelectionSource::FreshAssignment,
        &limited,
    ));
    assert!(account_accepts_concurrency_limit(
        2,
        PoolRoutingSelectionSource::StickyReuse,
        &limited,
    ));
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_uses_tag_layer_over_group_limit() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Group Tag Limit").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind("alpha")
        .execute(&pool)
        .await
        .expect("assign group name");

    let mut relaxed_rule = test_tag_routing_rule();
    relaxed_rule.concurrency_limit = 6;
    let relaxed_tag = insert_test_tag(&pool, "alpha-relaxed", &relaxed_rule)
        .await
        .expect("insert relaxed tag");

    let mut strict_rule = test_tag_routing_rule();
    strict_rule.concurrency_limit = 2;
    let strict_tag = insert_test_tag(&pool, "alpha-strict", &strict_rule)
        .await
        .expect("insert strict tag");

    sync_account_tag_links(
        &pool,
        account_id,
        &[relaxed_tag.summary.id, strict_tag.summary.id],
    )
    .await
    .expect("attach tags");

    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "alpha",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 4,
        },
    )
    .await
    .expect("save group metadata");
    drop(conn);

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(rule.concurrency_limit, 2);
    assert_eq!(rule.field_sources.concurrency_limit, "tag");
    assert_eq!(
        rule.source_tag_ids,
        vec![relaxed_tag.summary.id, strict_tag.summary.id]
    );
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_ignores_group_policy_for_api_key_transit()
 {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Transit Group Policy Guard").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind("legacy-transit")
        .execute(&pool)
        .await
        .expect("assign legacy transit group name");

    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "legacy-transit",
        UpstreamAccountGroupMetadata::default(),
    )
    .await
    .expect("save legacy transit group metadata");
    drop(conn);
    sqlx::query(
        "UPDATE pool_upstream_account_group_notes SET policy_priority_tier = 'fallback' WHERE group_name = ?1",
    )
    .bind("legacy-transit")
    .execute(&pool)
    .await
    .expect("seed legacy transit group policy");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load transit effective routing rule");

    assert_eq!(rule.priority_tier, TagPriorityTier::Normal);
    assert_eq!(rule.field_sources.priority_tier, "root");
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_reads_tag_available_models_from_db() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Tag Model Constraint").await;

    let mut tag_rule = test_tag_routing_rule();
    tag_rule.available_models = vec!["gpt-5.5".to_string()];
    let tag = insert_test_tag(&pool, "tag-model-constraint", &tag_rule)
        .await
        .expect("insert model tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach model tag");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(rule.available_models, vec!["gpt-5.5".to_string()]);
    assert!(rule.available_models_defined);
    assert_eq!(rule.field_sources.available_models, "tag");
    assert!(account_accepts_requested_model(Some("gpt-5.5"), &rule));
    assert!(!account_accepts_requested_model(Some("gpt-4.1"), &rule));
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_fails_closed_on_malformed_tag_models() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Malformed Tag Model Constraint").await;
    let now_iso = format_utc_iso(Utc::now());
    let tag_id: i64 = sqlx::query_scalar(
        r#"
            INSERT INTO pool_tags (
                name, system_key, protected, allow_cut_out, allow_cut_in, priority_tier,
                fast_mode_rewrite_mode, concurrency_limit, upstream_429_retry_enabled,
                upstream_429_max_retries, available_models_json, created_at, updated_at
            ) VALUES ('malformed-tag-models', NULL, 0, 1, 1, 'normal', 'keep_original', 0, 0, 0,
                      'not-json', ?1, ?1)
            RETURNING id
            "#,
    )
    .bind(&now_iso)
    .fetch_one(&pool)
    .await
    .expect("insert malformed tag");
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?3)
            "#,
    )
    .bind(account_id)
    .bind(tag_id)
    .bind(&now_iso)
    .execute(&pool)
    .await
    .expect("attach malformed tag");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert!(rule.available_models_defined);
    assert!(rule.available_models.is_empty());
    assert_eq!(rule.field_sources.available_models, "tag");
    assert!(!account_accepts_requested_model(Some("gpt-5.4"), &rule));
}

#[tokio::test]
pub(crate) async fn ensure_account_has_unsupported_model_tag_creates_generic_system_deny_tag() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Unsupported Model Learn").await;

    ensure_account_has_unsupported_model_tag(&pool, account_id, "gpt-5.4-mini")
        .await
        .expect("learn unsupported model deny");

    let row: (Option<String>, i64) = sqlx::query_as(
        r#"
            SELECT tag.system_key, tag.protected
            FROM pool_upstream_account_tags link
            INNER JOIN pool_tags tag ON tag.id = link.tag_id
            WHERE link.account_id = ?1
            "#,
    )
    .bind(account_id)
    .fetch_one(&pool)
    .await
    .expect("load linked deny tag");

    assert_eq!(row.0.as_deref(), Some("unsupported_model:gpt-5.4-mini"));
    assert_eq!(row.1, 1);
}

#[tokio::test]
pub(crate) async fn ensure_protected_system_tag_clears_legacy_editable_policy() {
    let pool = test_pool().await;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
            UPDATE pool_tags
            SET system_key = NULL,
                protected = 0,
                allow_cut_out = 0,
                allow_cut_in = 0,
                priority_tier = 'fallback',
                fast_mode_rewrite_mode = 'force_remove',
                concurrency_limit = 7,
                upstream_429_retry_enabled = 1,
                upstream_429_max_retries = 9,
                available_models_json = '["gpt-5.4"]',
                updated_at = ?2
            WHERE name = ?1
            "#,
    )
    .bind(GPT55_UNSUPPORTED_SYSTEM_TAG_NAME)
    .bind(&now_iso)
    .execute(&pool)
    .await
    .expect("prepare legacy tag");

    ensure_protected_system_tag(
        &pool,
        GPT55_UNSUPPORTED_SYSTEM_TAG_NAME,
        GPT55_UNSUPPORTED_SYSTEM_TAG_KEY,
    )
    .await
    .expect("promote legacy system tag");

    let row: (i64, i64, String, String, i64, i64, String) = sqlx::query_as(
        r#"
            SELECT allow_cut_out, allow_cut_in, priority_tier, fast_mode_rewrite_mode,
                   concurrency_limit, upstream_429_retry_enabled, available_models_json
            FROM pool_tags
            WHERE system_key = ?1
            "#,
    )
    .bind(GPT55_UNSUPPORTED_SYSTEM_TAG_KEY)
    .fetch_one(&pool)
    .await
    .expect("load promoted system tag");

    assert_eq!(row.0, 1);
    assert_eq!(row.1, 1);
    assert_eq!(row.2, "normal");
    assert_eq!(row.3, "keep_original");
    assert_eq!(row.4, 0);
    assert_eq!(row.5, 0);
    assert_eq!(row.6, "[\"gpt-5.4\"]");
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_applies_group_tag_account_overrides() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Layered Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = ?2,
                policy_allow_cut_in = 1,
                policy_fast_mode_rewrite_mode = 'force_remove',
                policy_upstream_429_retry_enabled = 1,
                policy_upstream_429_max_retries = 4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("layered")
    .execute(&pool)
    .await
    .expect("assign account override");

    let mut tag_rule = test_tag_routing_rule();
    tag_rule.allow_cut_in = false;
    tag_rule.priority_tier = TagPriorityTier::Fallback;
    tag_rule.fast_mode_rewrite_mode = TagFastModeRewriteMode::FillMissing;
    tag_rule.concurrency_limit = 3;
    tag_rule.upstream_429_retry_enabled = true;
    tag_rule.upstream_429_max_retries = 2;
    let tag = insert_test_tag(&pool, "layered-tag", &tag_rule)
        .await
        .expect("insert layered tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach layered tag");

    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "layered",
        UpstreamAccountGroupMetadata {
            note: None,
            bound_proxy_keys: vec![],
            node_shunt_enabled: false,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            concurrency_limit: 8,
        },
    )
    .await
    .expect("save legacy group metadata");
    drop(conn);
    sqlx::query(
        r#"
            UPDATE pool_upstream_account_group_notes
            SET policy_priority_tier = 'primary',
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_concurrency_limit = 5
            WHERE group_name = 'layered'
            "#,
    )
    .execute(&pool)
    .await
    .expect("save group policy override");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load layered effective routing rule");

    assert_eq!(rule.priority_tier, TagPriorityTier::Fallback);
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceRemove
    );
    assert_eq!(rule.concurrency_limit, 3);
    assert_eq!(rule.field_sources.concurrency_limit, "tag");
    assert!(rule.allow_cut_in);
    assert_eq!(rule.field_sources.allow_cut_in, "account");
    assert!(rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 4);
    assert_eq!(rule.field_sources.priority_tier, "tag");
    assert_eq!(rule.field_sources.fast_mode_rewrite_mode, "account");
    assert_eq!(rule.field_sources.upstream_429_retry, "account");
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_lets_tag_disable_group_retry() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Tag Retry Disable").await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(account_id)
        .bind("retry-group")
        .execute(&pool)
        .await
        .expect("assign group name");

    let tag_rule = test_tag_routing_rule();
    assert!(!tag_rule.upstream_429_retry_enabled);
    let tag = insert_test_tag(&pool, "retry-disabled-tag", &tag_rule)
        .await
        .expect("insert retry-disabled tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach retry-disabled tag");

    let mut conn = pool.acquire().await.expect("acquire metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "retry-group",
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
            SET policy_upstream_429_retry_enabled = 1,
                policy_upstream_429_max_retries = 5
            WHERE group_name = 'retry-group'
            "#,
    )
    .execute(&pool)
    .await
    .expect("save group retry policy");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert!(!rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 0);
    assert_eq!(rule.field_sources.upstream_429_retry, "tag");
}

#[tokio::test]
pub(crate) async fn load_effective_routing_rule_for_account_allows_account_block_override_to_clear_group()
 {
    let pool = test_pool().await;
    sqlx::query(
        r#"
            INSERT INTO pool_upstream_account_group_notes (
                group_name,
                note,
                policy_priority_tier,
                created_at,
                updated_at
            ) VALUES ('blocked-group', '', 'no_new', '2026-03-15T00:00:00Z', '2026-03-15T00:00:00Z')
            "#,
    )
    .execute(&pool)
    .await
    .expect("save group block policy");
    let account_id = insert_api_key_account(&pool, "Account Block Override").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET group_name = 'blocked-group',
                policy_priority_tier = 'normal'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&pool)
    .await
    .expect("save account routing policy");
    let tag = insert_test_tag(&pool, "block-tag", &test_tag_routing_rule())
        .await
        .expect("insert block tag");
    sync_account_tag_links(&pool, account_id, &[tag.summary.id])
        .await
        .expect("attach block tag");

    let rule = load_effective_routing_rule_for_account(&pool, account_id)
        .await
        .expect("load effective routing rule");

    assert_eq!(rule.priority_tier, TagPriorityTier::Normal);
    assert_eq!(rule.field_sources.priority_tier, "account");
}

#[tokio::test]
pub(crate) async fn update_upstream_account_preserves_account_policy_when_routing_rule_is_missing()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Preserve Account Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_allow_cut_in = 0,
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_upstream_429_retry_enabled = 1,
                policy_upstream_429_max_retries = 3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account policy");

    let detail = state
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
                note: Some("metadata only".to_string()),
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                bound_proxy_keys: OptionalField::Missing,
                enabled: Some(false),
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("metadata-only update");

    let rule = load_effective_routing_rule_for_account(&state.pool, account_id)
        .await
        .expect("load preserved policy");
    assert!(!rule.allow_cut_in);
    assert_eq!(rule.field_sources.allow_cut_in, "account");
    assert_eq!(
        rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceAdd
    );
    assert_eq!(rule.field_sources.fast_mode_rewrite_mode, "account");
    assert!(rule.upstream_429_retry_enabled);
    assert_eq!(rule.upstream_429_max_retries, 3);
    assert_eq!(rule.field_sources.upstream_429_retry, "account");
    assert!(!detail.summary.effective_routing_rule.allow_cut_in);
    assert_eq!(
        detail.summary.effective_routing_rule.fast_mode_rewrite_mode,
        TagFastModeRewriteMode::ForceAdd
    );
    assert_eq!(
        detail
            .summary
            .effective_routing_rule
            .field_sources
            .fast_mode_rewrite_mode,
        "account"
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_clears_individual_account_policy_override() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Clear Account Policy").await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET policy_allow_cut_in = 0,
                policy_fast_mode_rewrite_mode = 'force_add',
                policy_available_models_json = '[]'
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account policy");

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
                bound_proxy_keys: OptionalField::Missing,
                note: None,
                group_note: None,
                concurrency_limit: None,
                upstream_base_url: OptionalField::Missing,
                enabled: None,
                is_mother: None,
                api_key: None,
                local_primary_limit: None,
                local_secondary_limit: None,
                local_limit_unit: None,
                tag_ids: None,
                routing_rule: Some(UpdateGroupAccountRoutingRuleRequest {
                    allow_cut_out: OptionalField::Missing,
                    allow_cut_in: OptionalField::Null,
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
        .expect("clear account policy field");

    let stored = sqlx::query_as::<_, (Option<i64>, Option<String>, Option<String>)>(
            "SELECT policy_allow_cut_in, policy_fast_mode_rewrite_mode, policy_available_models_json FROM pool_upstream_accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_one(&state.pool)
        .await
        .expect("load stored policy");
    assert_eq!(stored.0, None);
    assert_eq!(stored.1.as_deref(), Some("force_add"));
    assert_eq!(stored.2.as_deref(), Some("[]"));
}

use super::*;
