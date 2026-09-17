#[test]
pub(crate) fn normalize_prompt_cache_conversation_limit_accepts_whitelist_values_only() {
    assert_eq!(normalize_prompt_cache_conversation_limit(None), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(20)), 20);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(50)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(100)), 100);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(10)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(200)), 50);
}

#[test]
pub(crate) fn normalize_prompt_cache_conversation_activity_hours_accepts_whitelist_values_only() {
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(None),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(1)),
        Some(1)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(3)),
        Some(3)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(6)),
        Some(6)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(12)),
        Some(12)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(24)),
        Some(24)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(2)),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_hours(Some(48)),
        None
    );
}

#[test]
pub(crate) fn normalize_prompt_cache_conversation_activity_minutes_accepts_precise_five_minutes_only()
 {
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(None),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(5)),
        Some(5)
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(1)),
        None
    );
    assert_eq!(
        normalize_prompt_cache_conversation_activity_minutes(Some(10)),
        None
    );
}

#[test]
pub(crate) fn resolve_prompt_cache_conversation_selection_rejects_mutually_exclusive_params() {
    let err = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: Some(20),
        activity_hours: Some(3),
        activity_minutes: None,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,

        blocked_binding_upstream_account_id: None,

        blocked_binding_constraint_source: None,
    })
    .expect_err("selection should reject mutually exclusive params");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(message.contains("mutually exclusive"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[test]
pub(crate) fn resolve_prompt_cache_conversation_selection_rejects_activity_hours_and_minutes_combo()
{
    let err = resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit: None,
        activity_hours: Some(3),
        activity_minutes: Some(5),
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,

        blocked_binding_upstream_account_id: None,

        blocked_binding_constraint_source: None,
    })
    .expect_err("selection should reject mixed hour and minute windows");

    match err {
        ApiError::BadRequest(inner) => {
            let message = inner.to_string();
            assert!(message.contains("mutually exclusive"));
        }
        other => panic!("expected bad request, got {other:?}"),
    }
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversations_groups_recent_keys_and_uses_history_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    seed_prompt_cache_conversation_history(&state.pool, now).await;

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before count-mode prompt-cache read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache conversation stats should succeed");

    assert_eq!(
        response.selection_mode,
        PromptCacheConversationSelectionMode::Count
    );
    assert_eq!(response.selected_limit, Some(20));
    assert_eq!(response.selected_activity_hours, None);
    assert_eq!(
        response.implicit_filter.kind,
        Some(PromptCacheConversationImplicitFilterKind::InactiveOutside24h)
    );
    assert_eq!(response.implicit_filter.filtered_count, 1);
    assert_eq!(response.conversations.len(), 2);
    assert_eq!(response.conversations[0].prompt_cache_key, "pck-b");
    assert_eq!(response.conversations[1].prompt_cache_key, "pck-a");

    let key_a = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-a")
        .expect("pck-a should be included");
    assert_eq!(key_a.request_count, 3);
    assert_eq!(key_a.total_tokens, 150);
    assert!((key_a.total_cost - 1.5).abs() < 1e-9);
    assert_eq!(key_a.last24h_requests.len(), 2);
    assert_eq!(key_a.last24h_requests[0].request_tokens, 20);
    assert_eq!(key_a.last24h_requests[0].cumulative_tokens, 20);
    assert!(key_a.last24h_requests[0].is_success);
    assert_eq!(key_a.last24h_requests[0].outcome, "success");
    assert_eq!(key_a.last24h_requests[1].request_tokens, 30);
    assert_eq!(key_a.last24h_requests[1].cumulative_tokens, 50);
    assert!(!key_a.last24h_requests[1].is_success);
    assert_eq!(key_a.last24h_requests[1].outcome, "failure");
}

struct PromptCacheInvocation<'a> {
    invoke_id: &'a str,
    occurred_at: DateTime<Utc>,
    key: Option<&'a str>,
    status: &'a str,
    total_tokens: i64,
    cost: f64,
}

async fn seed_prompt_cache_conversation_history(pool: &Pool<Sqlite>, now: DateTime<Utc>) {
    for row in [
        PromptCacheInvocation {
            invoke_id: "pck-a-history",
            occurred_at: now - ChronoDuration::hours(48),
            key: Some("pck-a"),
            status: "success",
            total_tokens: 100,
            cost: 1.0,
        },
        PromptCacheInvocation {
            invoke_id: "pck-a-24h-1",
            occurred_at: now - ChronoDuration::hours(2),
            key: Some("pck-a"),
            status: "success",
            total_tokens: 20,
            cost: 0.2,
        },
        PromptCacheInvocation {
            invoke_id: "pck-a-24h-2",
            occurred_at: now - ChronoDuration::hours(1),
            key: Some("pck-a"),
            status: "failed",
            total_tokens: 30,
            cost: 0.3,
        },
        PromptCacheInvocation {
            invoke_id: "pck-b-24h-1",
            occurred_at: now - ChronoDuration::hours(10),
            key: Some("pck-b"),
            status: "success",
            total_tokens: 10,
            cost: 0.1,
        },
        PromptCacheInvocation {
            invoke_id: "pck-c-history",
            occurred_at: now - ChronoDuration::hours(72),
            key: Some("pck-c"),
            status: "success",
            total_tokens: 8,
            cost: 0.08,
        },
        PromptCacheInvocation {
            invoke_id: "pck-missing-24h",
            occurred_at: now - ChronoDuration::minutes(40),
            key: None,
            status: "success",
            total_tokens: 999,
            cost: 9.99,
        },
    ] {
        insert_prompt_cache_invocation(pool, row).await;
    }
}

async fn insert_prompt_cache_invocation(pool: &Pool<Sqlite>, row: PromptCacheInvocation<'_>) {
    let payload = row
        .key
        .map(|key| json!({ "promptCacheKey": key }).to_string())
        .unwrap_or_else(|| "{}".to_string());
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(row.invoke_id)
    .bind(format_naive(
        row.occurred_at.with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(row.status)
    .bind(row.total_tokens)
    .bind(row.cost)
    .bind(payload)
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert invocation row");
}

#[tokio::test]
pub(crate) async fn prompt_cache_last24h_requests_keep_null_status_rows_neutral() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_at = format_naive(
        (now - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-neutral-success")
    .bind(occurred_at.clone())
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(20_i64)
    .bind(0.2_f64)
    .bind(json!({ "promptCacheKey": "pck-neutral" }).to_string())
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert success prompt cache row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("pck-neutral-null-status")
    .bind(format_naive(
        (now - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(15_i64)
    .bind(0.15_f64)
    .bind(json!({ "promptCacheKey": "pck-neutral" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("")
    .execute(&state.pool)
    .await
    .expect("insert null-status prompt cache row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before neutral status read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache neutral conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-neutral")
        .expect("neutral prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 2);
    assert_eq!(conversation.last24h_requests[0].outcome, "success");
    assert_eq!(conversation.last24h_requests[1].status, "unknown");
    assert!(!conversation.last24h_requests[1].is_success);
    assert_eq!(conversation.last24h_requests[1].outcome, "neutral");
}

#[tokio::test]
pub(crate) async fn prompt_cache_last24h_requests_treat_running_rows_with_failure_class_as_failures()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-running-failure")
    .bind(format_naive(
        (now - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(11_i64)
    .bind(0.11_f64)
    .bind(json!({ "promptCacheKey": "pck-running-failure" }).to_string())
    .bind("{}")
    .bind("service_failure")
    .bind("upstream stream error")
    .execute(&state.pool)
    .await
    .expect("insert running prompt cache failure row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before running failure read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache running failure conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-running-failure")
        .expect("running failure prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "running");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
pub(crate) async fn prompt_cache_last24h_requests_treat_running_rows_with_error_text_as_failures() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-running-error-text")
    .bind(format_naive(
        (now - ChronoDuration::minutes(4))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("running")
    .bind(9_i64)
    .bind(0.09_f64)
    .bind(json!({ "promptCacheKey": "pck-running-error-text" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("downstream closed while streaming upstream response")
    .execute(&state.pool)
    .await
    .expect("insert running prompt cache row with error text");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before running error-text read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache running error-text conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-running-error-text")
        .expect("running error-text prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "running");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
pub(crate) async fn prompt_cache_last24h_requests_treat_pending_rows_with_failure_kind_as_failures()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message, failure_kind
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
    )
    .bind("pck-pending-failure-kind")
    .bind(format_naive(
        (now - ChronoDuration::minutes(3))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("pending")
    .bind(7_i64)
    .bind(0.07_f64)
    .bind(
        json!({
            "promptCacheKey": "pck-pending-failure-kind",
            "downstreamErrorMessage": "pool upstream responded with 502",
        })
        .to_string(),
    )
    .bind("{}")
    .bind("none")
    .bind::<Option<&str>>(None)
    .bind(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    .execute(&state.pool)
    .await
    .expect("insert pending prompt cache row with failure kind");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before pending failure-kind read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache pending failure-kind conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-pending-failure-kind")
        .expect("pending failure-kind prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "pending");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
pub(crate) async fn prompt_cache_last24h_requests_keep_status_only_http_failures_marked_as_failures()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response,
            failure_class, error_message
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind("pck-http-status-only-failure")
    .bind(format_naive(
        (now - ChronoDuration::minutes(2))
            .with_timezone(&Shanghai)
            .naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind("http_500")
    .bind(5_i64)
    .bind(0.05_f64)
    .bind(json!({ "promptCacheKey": "pck-http-status-only-failure" }).to_string())
    .bind("{}")
    .bind("none")
    .bind("")
    .execute(&state.pool)
    .await
    .expect("insert http-status-only prompt cache failure row");

    sync_hourly_rollups_from_live_tables(&state.pool)
        .await
        .expect("materialize prompt cache rollups before http-status-only read");

    let Json(response) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery {
            limit: Some(20),
            activity_hours: None,
            activity_minutes: None,
            page_size: None,
            cursor: None,
            snapshot_at: None,
            detail: None,
            recent_invocation_limit: None,

            blocked_binding_upstream_account_id: None,

            blocked_binding_constraint_source: None,
        }),
    )
    .await
    .expect("prompt cache http-status-only failure conversation stats should succeed");

    let conversation = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-http-status-only-failure")
        .expect("http-status-only prompt cache conversation should exist");
    assert_eq!(conversation.last24h_requests.len(), 1);
    assert_eq!(conversation.last24h_requests[0].status, "http_500");
    assert!(!conversation.last24h_requests[0].is_success);
    assert_eq!(conversation.last24h_requests[0].outcome, "failure");
}

#[tokio::test]
pub(crate) async fn prompt_cache_conversation_binding_patch_is_mutually_exclusive_and_clearable() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let group_name = "prompt-cache-bindings-api-group";
    ensure_test_group_binding(&state.pool, group_name, None).await;
    let account_id = insert_test_pool_oauth_account(
        &state,
        "Prompt Cache Binding OAuth",
        "oauth-prompt-cache-binding",
    )
    .await;
    set_test_account_group_name(&state.pool, account_id, Some(group_name)).await;
    let unselectable_account_id = insert_test_pool_oauth_account(
        &state,
        "Prompt Cache Binding Unselectable OAuth",
        "oauth-prompt-cache-binding-unselectable",
    )
    .await;
    set_test_account_group_name(&state.pool, unselectable_account_id, Some(group_name)).await;
    sqlx::query("UPDATE pool_upstream_accounts SET encrypted_credentials = NULL WHERE id = ?1")
        .bind(unselectable_account_id)
        .execute(&state.pool)
        .await
        .expect("make prompt cache binding target unselectable");
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET policy_allow_cut_out = 0,
            policy_fast_mode_rewrite_mode = 'force_remove',
            policy_image_tool_rewrite_mode = 'fill_missing',
            policy_codex_imagegen_rewrite_mode = 'fill_missing',
            policy_available_models_json = '["gpt-5.1-codex-mini"]'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed account policy for inherited conversation response");

    exercise_binding_validation_and_lifecycle(
        state,
        group_name,
        account_id,
        unselectable_account_id,
        "prompt-cache-binding-api+literal&part=value",
    )
    .await;
}

async fn exercise_binding_validation_and_lifecycle(
    state: Arc<AppState>,
    group_name: &str,
    account_id: i64,
    unselectable_account_id: i64,
    prompt_cache_key: &str,
) {
    let both_payload: UpdatePromptCacheConversationBindingRequest = serde_json::from_value(json!({
        "bindingKind": "group",
        "groupName": group_name,
        "upstreamAccountId": account_id,
    }))
    .expect("deserialize mutually exclusive binding payload");
    let both_err = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(format!("  {prompt_cache_key}  ")),
        Json(both_payload),
    )
    .await
    .expect_err("mutually exclusive binding payload should fail");
    assert!(matches!(both_err, ApiError::BadRequest(_)));

    let group_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "group",
            "groupName": group_name,
        }))
        .expect("deserialize group binding payload");
    let Json(group_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(format!("  {prompt_cache_key}  ")),
        Json(group_payload),
    )
    .await
    .expect("group binding should save");
    assert_eq!(group_response.prompt_cache_key, prompt_cache_key);
    assert_eq!(group_response.binding_kind, "group");
    assert_eq!(group_response.group_name.as_deref(), Some(group_name));
    assert_eq!(group_response.upstream_account_id, None);

    assert_inherited_account_binding(state, account_id, unselectable_account_id, prompt_cache_key)
        .await;
}

async fn assert_inherited_account_binding(
    state: Arc<AppState>,
    account_id: i64,
    unselectable_account_id: i64,
    prompt_cache_key: &str,
) {
    let inherited_account_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
        }))
        .expect("deserialize inherited account binding payload");
    let Json(inherited_account_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(inherited_account_payload),
    )
    .await
    .expect("account binding without policy override should save");
    assert_eq!(
        inherited_account_response.allow_switch_upstream,
        Some(false)
    );
    assert_eq!(
        inherited_account_response.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceRemove)
    );
    assert_eq!(
        inherited_account_response.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::FillMissing)
    );
    assert_eq!(
        inherited_account_response.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::FillMissing)
    );
    assert_eq!(
        inherited_account_response.available_models,
        Some(vec!["gpt-5.1-codex-mini".to_string()])
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .allow_switch_upstream,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .fast_mode_rewrite_mode,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .image_tool_rewrite_mode,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .codex_imagegen_rewrite_mode,
        "account"
    );
    assert_eq!(
        inherited_account_response
            .policy_field_sources
            .available_models,
        "account"
    );

    assert_explicit_account_binding(state, account_id, unselectable_account_id, prompt_cache_key)
        .await;
}

async fn assert_explicit_account_binding(
    state: Arc<AppState>,
    account_id: i64,
    unselectable_account_id: i64,
    prompt_cache_key: &str,
) {
    let unselectable_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": unselectable_account_id,
        }))
        .expect("deserialize unselectable account binding payload");
    let unselectable_err = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(unselectable_payload),
    )
    .await
    .expect_err("unselectable account binding should fail");
    assert!(matches!(unselectable_err, ApiError::BadRequest(_)));

    let account_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
            "allowSwitchUpstream": true,
            "fastModeRewriteMode": "force_add",
            "imageToolRewriteMode": "force_remove",
            "codexImagegenRewriteMode": "force_add",
            "availableModels": ["gpt-5.1-codex-max", "gpt-5.1-codex-max", "gpt-5.1-codex-mini"],
            "forwardProxyKey": "__direct__",
        }))
        .expect("deserialize account binding payload");
    let Json(account_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(account_payload),
    )
    .await
    .expect("account binding should save");
    assert_eq!(account_response.binding_kind, "upstreamAccount");
    assert_eq!(account_response.group_name, None);
    assert_eq!(account_response.upstream_account_id, Some(account_id));
    assert_eq!(account_response.allow_switch_upstream, Some(true));
    assert_eq!(
        account_response.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceAdd)
    );
    assert_eq!(
        account_response.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::ForceRemove)
    );
    assert_eq!(
        account_response.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::ForceAdd)
    );
    assert_eq!(
        account_response.available_models,
        Some(vec![
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.1-codex-mini".to_string()
        ])
    );
    assert_eq!(
        account_response.forward_proxy_key.as_deref(),
        Some("__direct__")
    );

    assert_stored_conversation_override(state, account_id, prompt_cache_key).await;
}

async fn assert_stored_conversation_override(
    state: Arc<AppState>,
    account_id: i64,
    prompt_cache_key: &str,
) {
    let conversation_override =
        load_prompt_cache_conversation_routing_override(&state.pool, Some(prompt_cache_key))
            .await
            .expect("conversation override should load")
            .expect("conversation override should exist");
    assert_eq!(conversation_override.allow_switch_upstream, Some(true));
    assert_eq!(
        conversation_override.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceAdd)
    );
    assert_eq!(
        conversation_override.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::ForceRemove)
    );
    assert_eq!(
        conversation_override.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::ForceAdd)
    );
    assert_eq!(
        conversation_override.available_models,
        Some(vec![
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.1-codex-mini".to_string()
        ])
    );
    assert_eq!(
        conversation_override.forward_proxy_key.as_deref(),
        Some("__direct__")
    );

    assert_empty_models_and_sticky_route(state, account_id, prompt_cache_key).await;
}

async fn assert_empty_models_and_sticky_route(
    state: Arc<AppState>,
    account_id: i64,
    prompt_cache_key: &str,
) {
    let empty_models_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "upstreamAccount",
            "upstreamAccountId": account_id,
            "availableModels": [],
        }))
        .expect("deserialize empty available models payload");
    let Json(empty_models_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(empty_models_payload),
    )
    .await
    .expect("empty available models allowlist should save and reject all models");
    assert_eq!(empty_models_response.available_models, Some(Vec::new()));
    assert_eq!(
        empty_models_response.available_models_mode,
        Some(AvailableModelsMode::Allowlist)
    );
    let sticky_account_id: i64 =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("account binding should update sticky route");
    assert_eq!(sticky_account_id, account_id);

    assert_cleared_binding_inherits_account_policy(state, account_id, prompt_cache_key).await;
}

async fn assert_cleared_binding_inherits_account_policy(
    state: Arc<AppState>,
    account_id: i64,
    prompt_cache_key: &str,
) {
    let clear_payload: UpdatePromptCacheConversationBindingRequest =
        serde_json::from_value(json!({
            "bindingKind": "none",
            "allowSwitchUpstream": null,
            "fastModeRewriteMode": null,
            "imageToolRewriteMode": null,
            "codexImagegenRewriteMode": null,
            "availableModels": null,
            "forwardProxyKey": null,
        }))
        .expect("deserialize clear binding payload");
    let Json(clear_response) = patch_prompt_cache_conversation_binding(
        State(state.clone()),
        AxumPath(prompt_cache_key.to_string()),
        Json(clear_payload),
    )
    .await
    .expect("clear binding should delete row");
    assert_eq!(clear_response.binding_kind, "none");
    assert_eq!(clear_response.allow_switch_upstream, Some(false));
    assert_eq!(
        clear_response.fast_mode_rewrite_mode,
        Some(TagFastModeRewriteMode::ForceRemove)
    );
    assert_eq!(
        clear_response.image_tool_rewrite_mode,
        Some(ImageToolRewriteMode::FillMissing)
    );
    assert_eq!(
        clear_response.codex_imagegen_rewrite_mode,
        Some(CodexImagegenRewriteMode::FillMissing)
    );
    assert_eq!(
        clear_response.available_models,
        Some(vec!["gpt-5.1-codex-mini".to_string()])
    );
    assert_eq!(
        clear_response.forward_proxy_key.as_deref(),
        Some("__direct__")
    );
    assert_eq!(
        clear_response.policy_field_sources.allow_switch_upstream,
        "account"
    );
    assert_eq!(
        clear_response.policy_field_sources.fast_mode_rewrite_mode,
        "account"
    );
    assert_eq!(
        clear_response.policy_field_sources.image_tool_rewrite_mode,
        "account"
    );
    assert_eq!(
        clear_response
            .policy_field_sources
            .codex_imagegen_rewrite_mode,
        "account"
    );
    assert_eq!(
        clear_response.policy_field_sources.available_models,
        "account"
    );
    let cleared_conversation_override =
        load_prompt_cache_conversation_routing_override(&state.pool, Some(prompt_cache_key))
            .await
            .expect("cleared conversation override should load");
    assert!(
        cleared_conversation_override.is_none(),
        "null policy fields should clear stored conversation overrides"
    );
    let sticky_account_id_after_clear: i64 =
        sqlx::query_scalar("SELECT account_id FROM pool_sticky_routes WHERE sticky_key = ?1")
            .bind(prompt_cache_key)
            .fetch_one(&state.pool)
            .await
            .expect("clearing a binding should leave existing sticky route intact");
    assert_eq!(sticky_account_id_after_clear, account_id);
    let Json(get_response) =
        get_prompt_cache_conversation_binding(State(state), AxumPath(prompt_cache_key.to_string()))
            .await
            .expect("get cleared binding");
    assert_eq!(get_response.binding_kind, "none");
}

use super::*;
