use super::*;

#[tokio::test]
async fn image_intent_route_success_learns_supported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Success Learns Supported").await;

    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        account_id,
        Utc::now(),
        Some("sticky-image-supported"),
        Some("invk_image_supported"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record image route success");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after image success")
        .expect("row exists after image success");
    assert_eq!(
        row.response_endpoint_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(row.chat_completions_capability.as_deref(), Some("unknown"));

    let direct_account_id =
        insert_oauth_account(&pool, "Direct Image Success Learns Supported").await;
    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        direct_account_id,
        Utc::now(),
        Some("sticky-direct-image-supported"),
        Some("invk_direct_image_supported"),
        "/v1/images/generations",
        ImageIntent::DirectImage,
    )
    .await
    .expect("record direct image route success");

    let direct_row = load_upstream_account_row(&pool, direct_account_id)
        .await
        .expect("load row after direct image success")
        .expect("row exists after direct image success");
    assert_eq!(
        direct_row.image_endpoint_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        direct_row.response_endpoint_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
async fn chat_completions_route_learning_stays_on_chat_axis() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Chat Success Learns Supported").await;

    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        account_id,
        Utc::now(),
        Some("sticky-chat-supported"),
        Some("invk_chat_supported"),
        "/v1/chat/completions",
        ImageIntent::No,
    )
    .await
    .expect("record chat completions route success");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after chat success")
        .expect("row exists after chat success");
    assert_eq!(
        row.chat_completions_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(row.image_endpoint_capability.as_deref(), Some("unknown"));

    let unsupported_account_id =
        insert_oauth_account(&pool, "Chat Failure Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        unsupported_account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-chat-unsupported"),
        StatusCode::NOT_FOUND,
        "pool upstream responded with 404: unsupported endpoint /v1/chat/completions for this account",
        Some("invk_chat_unsupported"),
        "/v1/chat/completions",
        ImageIntent::No,
    )
    .await
    .expect("record explicit unsupported chat completions failure");

    let unsupported_row = load_upstream_account_row(&pool, unsupported_account_id)
        .await
        .expect("load row after unsupported chat failure")
        .expect("row exists after unsupported chat failure");
    assert_eq!(
        unsupported_row.chat_completions_capability.as_deref(),
        Some("unsupported")
    );
    assert_eq!(
        unsupported_row.response_endpoint_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
async fn standalone_search_learning_is_api_key_only_and_route_specific() {
    let pool = test_pool().await;
    let supported_id = insert_api_key_account(&pool, "Search Success Learns Supported").await;
    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        supported_id,
        Utc::now(),
        None,
        Some("invk_search_supported"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record search route success");
    let supported = load_upstream_account_row(&pool, supported_id)
        .await
        .expect("load supported search row")
        .expect("supported search row exists");
    assert_eq!(
        supported.standalone_search_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        supported.response_endpoint_capability.as_deref(),
        Some("unknown")
    );

    let unsupported_id = insert_api_key_account(&pool, "Search 404 Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        unsupported_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::NOT_FOUND,
        "pool upstream responded with 404",
        Some("invk_search_unsupported"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record bare search 404");
    let unsupported = load_upstream_account_row(&pool, unsupported_id)
        .await
        .expect("load unsupported search row")
        .expect("unsupported search row exists");
    assert_eq!(
        unsupported.standalone_search_capability.as_deref(),
        Some("unsupported")
    );

    let ambiguous_id = insert_api_key_account(&pool, "Search 400 Keeps Unknown").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        ambiguous_id,
        UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX,
        false,
        None,
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: request body is invalid",
        Some("invk_search_ambiguous"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record ambiguous search 400");
    let ambiguous = load_upstream_account_row(&pool, ambiguous_id)
        .await
        .expect("load ambiguous search row")
        .expect("ambiguous search row exists");
    assert_eq!(
        ambiguous.standalone_search_capability.as_deref(),
        Some("unknown")
    );

    let oauth_id = insert_oauth_account(&pool, "OAuth Search Does Not Learn").await;
    record_pool_route_success_for_endpoint_with_image_intent(
        &pool,
        oauth_id,
        Utc::now(),
        None,
        Some("invk_oauth_search"),
        "/v1/alpha/search",
        ImageIntent::No,
    )
    .await
    .expect("record OAuth search route success");
    let oauth = load_upstream_account_row(&pool, oauth_id)
        .await
        .expect("load OAuth search row")
        .expect("OAuth search row exists");
    assert_eq!(
        oauth.standalone_search_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
async fn standalone_search_override_round_trips_through_account_detail() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Search Override Round Trip").await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET standalone_search_capability = 'unsupported',
            standalone_search_capability_observed_at = '2026-08-05T15:00:00Z',
            standalone_search_capability_reason = 'pool upstream responded with 404'
        WHERE id = ?1
        "#,
    )
    .bind(account_id)
    .execute(&state.pool)
    .await
    .expect("seed unsupported search capability");

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Value(
                    "supported".to_string(),
                ),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("save search capability override");

    assert_eq!(
        detail.summary.standalone_search_capability.observed,
        CapabilitySupport::Unsupported
    );
    assert_eq!(
        detail.summary.standalone_search_capability.override_value,
        Some(CapabilitySupport::Supported)
    );
    assert_eq!(
        detail.summary.standalone_search_capability.effective,
        CapabilitySupport::Supported
    );
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after search override")
        .expect("account exists after search override");
    assert_eq!(
        row.policy_standalone_search_capability_override.as_deref(),
        Some("supported")
    );

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Null,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("clear search capability override");

    assert_eq!(
        detail.summary.standalone_search_capability.override_value,
        None
    );
    assert_eq!(
        detail.summary.standalone_search_capability.effective,
        CapabilitySupport::Unsupported
    );
    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after clearing search override")
        .expect("account exists after clearing search override");
    assert_eq!(row.policy_standalone_search_capability_override, None);
}

#[tokio::test]
async fn standalone_search_override_is_rejected_for_oauth_accounts() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "OAuth Search Override Rejected").await;

    let err = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Value(
                    "supported".to_string(),
                ),
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect_err("OAuth accounts must reject standalone search overrides");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(err.1.contains("only supported for API key accounts"));

    let err = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            account_id,
            UpdateUpstreamAccountRequest {
                standalone_search_capability_override: OptionalField::Null,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect_err("OAuth accounts must reject clearing standalone search overrides");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);

    let row = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load OAuth account after rejected override")
        .expect("OAuth account exists after rejected override");
    assert_eq!(row.policy_standalone_search_capability_override, None);
}

#[tokio::test]
async fn image_intent_explicit_unsupported_failure_learns_unsupported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Failure Learns Unsupported").await;

    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-image-unsupported"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: unsupported tool: image_generation is not supported by this account",
        Some("invk_image_unsupported"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record explicit unsupported image failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after unsupported image failure")
        .expect("row exists after unsupported image failure");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("unsupported")
    );

    let direct_account_id =
        insert_oauth_account(&pool, "Direct Image Failure Learns Unsupported").await;
    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        direct_account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-direct-image-unsupported"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: No available channel for model gpt-image-1 under group default",
        Some("invk_direct_image_unsupported"),
        "/v1/images/generations",
        ImageIntent::DirectImage,
    )
    .await
    .expect("record explicit unsupported direct image failure");

    let direct_row = load_upstream_account_row(&pool, direct_account_id)
        .await
        .expect("load row after unsupported direct image failure")
        .expect("row exists after unsupported direct image failure");
    assert_eq!(
        direct_row.image_endpoint_capability.as_deref(),
        Some("unsupported")
    );
}

#[tokio::test]
async fn stale_capability_observations_cannot_overwrite_newer_results() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Fenced Capability Observations").await;
    let newer = "2026-09-13T10:00:01.000Z";
    let older = "2026-09-13T10:00:00.000Z";

    record_capability_observation_with_observed_at(
        &pool,
        account_id,
        UpstreamCapabilityAxis::ResponseEndpoint,
        CapabilitySupport::Supported,
        Some("newer success"),
        Some(newer),
    )
    .await
    .expect("record newer capability observation");
    record_capability_observation_with_observed_at(
        &pool,
        account_id,
        UpstreamCapabilityAxis::ResponseEndpoint,
        CapabilitySupport::Unsupported,
        Some("older failure"),
        Some(older),
    )
    .await
    .expect("record stale capability observation");

    record_compact_support_observation_with_observed_at(
        &pool,
        account_id,
        COMPACT_SUPPORT_STATUS_SUPPORTED,
        Some("newer success"),
        Some(newer),
    )
    .await
    .expect("record newer compact observation");
    record_compact_support_observation_with_observed_at(
        &pool,
        account_id,
        COMPACT_SUPPORT_STATUS_UNSUPPORTED,
        Some("older failure"),
        Some(older),
    )
    .await
    .expect("record stale compact observation");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load fenced observation row")
        .expect("fenced observation row exists");
    assert_eq!(
        row.response_endpoint_capability.as_deref(),
        Some(CapabilitySupport::Supported.as_str())
    );
    assert_eq!(
        row.response_endpoint_capability_observed_at.as_deref(),
        Some(newer)
    );
    assert_eq!(
        row.compact_support_status.as_deref(),
        Some(COMPACT_SUPPORT_STATUS_SUPPORTED)
    );
    assert_eq!(row.compact_support_observed_at.as_deref(), Some(newer));
}

#[tokio::test]
async fn image_intent_validation_failure_does_not_learn_unsupported_capability() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Image Validation Failure Keeps Unknown").await;

    record_pool_route_http_failure_for_endpoint_with_image_intent(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX,
        false,
        Some("sticky-image-invalid-payload"),
        StatusCode::BAD_REQUEST,
        "pool upstream responded with 400: invalid image size: width must be divisible by 64",
        Some("invk_image_invalid_payload"),
        "/v1/responses",
        ImageIntent::Yes,
    )
    .await
    .expect("record image validation failure");

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after image validation failure")
        .expect("row exists after image validation failure");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(
        row.response_image_tool_capability.as_deref(),
        Some("unknown")
    );
}

#[tokio::test]
async fn mark_account_sync_success_preserves_route_cooldown_state() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Cooldown OAuth").await;
    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;

    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before sync")
        .expect("row exists before sync");
    mark_account_sync_success(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::PreserveFailureState,
    )
    .await
    .expect("mark sync success");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after sync")
        .expect("row exists after sync");

    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_synced_at.is_some());
    assert!(after.last_successful_sync_at.is_some());
    assert_eq!(after.last_route_failure_at, before.last_route_failure_at);
    assert_eq!(
        after.last_route_failure_kind,
        before.last_route_failure_kind
    );
    assert_eq!(after.cooldown_until, before.cooldown_until);
    assert_eq!(
        after.consecutive_route_failures,
        before.consecutive_route_failures
    );
}

#[tokio::test]
async fn mark_account_sync_success_clears_hard_unavailable_state_when_requested() {
    let pool = test_pool().await;
    let account_id = insert_oauth_account(&pool, "Recovered OAuth").await;
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
        UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL,
        SyncSuccessRouteState::ClearFailureState,
    )
    .await
    .expect("mark sync success");

    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after sync success")
        .expect("row exists after sync success");
    assert_eq!(after.status, UPSTREAM_ACCOUNT_STATUS_ACTIVE);
    assert!(after.last_error.is_none());
    assert!(after.last_route_failure_kind.is_none());
    assert!(after.cooldown_until.is_none());
    assert_eq!(
        after.last_action.as_deref(),
        Some(UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED)
    );
}

#[tokio::test]
async fn sync_api_key_account_preserves_route_cooldown_state() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Cooldown API Key").await;
    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");

    sync_api_key_account(&pool, &row, SyncCause::Manual)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    );
    assert!(after.cooldown_until.is_some());
    assert_eq!(after.consecutive_route_failures, 1);
}

#[tokio::test]
async fn sync_api_key_account_keeps_hard_unavailable_accounts_blocked() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, "Blocked API Key").await;
    seed_hard_unavailable_route_failure(
        &pool,
        account_id,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        Some(429),
    )
    .await;
    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load api key row")
        .expect("api key row exists");

    sync_api_key_account(&pool, &row, SyncCause::Manual)
        .await
        .expect("sync api key account");
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after api key sync")
        .expect("row exists after api key sync");

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
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED)
    );
    assert_eq!(
        after.last_route_failure_kind.as_deref(),
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED)
    );
}
