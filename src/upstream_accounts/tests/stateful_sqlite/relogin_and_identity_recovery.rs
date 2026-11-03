use super::*;

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_relogin_repairs() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
    let _sibling_id = insert_oauth_account(&state.pool, "Edited Relogin").await;
    let relogin = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Edited Relogin".to_string()),
            email: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            account_id: Some(account_id),
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create relogin session")
    .0;

    let pending_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load relogin session")
        .expect("relogin session should exist");
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "relogin-access".to_string(),
            refresh_token: Some("relogin-refresh".to_string()),
            id_token: test_id_token(
                "relogin@example.com",
                Some("org_test"),
                Some("user_test"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    record_upstream_account_refresh_failure(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
        "test",
        "refresh token revoked",
        "refresh_token_invalid",
        "Refresh Token 已失效，需要重新授权。",
        false,
        Some(StatusCode::BAD_REQUEST),
    )
    .await
    .expect("seed stale reauthorization state");

    let completed_account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: "Edited Relogin".to_string(),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims("relogin@example.com", Some("org_test"), Some("user_test")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist relogin callback");
    assert_eq!(completed_account_id, account_id);

    let refresh_status = load_upstream_account_refresh_status(
        &state.pool,
        account_id,
        UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS,
    )
    .await
    .expect("load callback refresh state");
    assert_eq!(
        refresh_status.state,
        UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED
    );
    assert_eq!(refresh_status.source.as_deref(), Some("oauth_callback"));

    let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load completed relogin session")
        .expect("completed relogin session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(
        completed_session.updated_at,
        completed_session.consumed_at.clone().unwrap()
    );

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&relogin.updated_at).expect("valid updated_at header"),
    );
    let err = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
        AxumPath(relogin.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Edited Relogin".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("edited-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("edited note".to_string()),
            group_note: OptionalField::Value("edited group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("completed relogin repair should be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load relogin target after rejected repair")
        .expect("relogin target should exist");
    assert_eq!(account.display_name, "Relogin Target");
    assert_ne!(account.group_name.as_deref(), Some("edited-group"));
    assert_ne!(account.note.as_deref(), Some("edited note"));
}

#[tokio::test]
async fn relogin_same_identity_skips_sibling_display_name_conflict() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_test_group_binding(&state.pool, test_required_group_name()).await;
    let mut tx = state.pool.begin().await.expect("begin tx");
    let target_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Shared OAuth",
            chosen_email: Some("target-manual@example.com".to_string()),
            verified_email: Some("target-verified@example.com".to_string()),
            group_name: Some(test_required_group_name().to_string()),
            is_mother: false,
            note: Some("keep target note".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "target-verified@example.com",
                Some("org_target"),
                Some("user_target"),
            ),
            encrypted_credentials: "encrypted-target-old".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert target oauth");
    upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Shared OAuth",
            chosen_email: Some("sibling@example.com".to_string()),
            verified_email: Some("sibling@example.com".to_string()),
            group_name: Some(test_required_group_name().to_string()),
            is_mother: false,
            note: Some("sibling".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "sibling@example.com",
                Some("org_sibling"),
                Some("user_sibling"),
            ),
            encrypted_credentials: "encrypted-sibling".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert sibling oauth");
    tx.commit().await.expect("commit tx");

    let relogin = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: None,
            email: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            account_id: Some(target_id),
            tag_ids: vec![],
            is_mother: None,
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create relogin session")
    .0;
    let session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load relogin session")
        .expect("relogin session should exist");
    let completed_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            session,
            display_name: "Shared OAuth".to_string(),
            chosen_email: Some("target-manual@example.com".to_string()),
            verified_email: Some("target-new@example.com".to_string()),
            claims: test_claims(
                "target-new@example.com",
                Some("org_target"),
                Some("user_target"),
            ),
            encrypted_credentials: "encrypted-target-new".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("same identity relogin should update");
    assert_eq!(completed_id, target_id);

    let account = load_upstream_account_row(&state.pool, target_id)
        .await
        .expect("load target")
        .expect("target exists");
    assert_eq!(account.display_name, "Shared OAuth");
    assert_eq!(account.email.as_deref(), Some("target-manual@example.com"));
    assert_eq!(
        account.verified_email.as_deref(),
        Some("target-new@example.com")
    );
    assert_eq!(account.chatgpt_account_id.as_deref(), Some("org_target"));
    assert_eq!(account.chatgpt_user_id.as_deref(), Some("user_target"));
    assert_eq!(
        account.encrypted_credentials.as_deref(),
        Some("encrypted-target-new")
    );
    let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load completed session")
        .expect("completed session exists");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
}

#[tokio::test]
async fn relogin_different_identity_requires_confirmation_then_preserves_display_fields() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_test_group_binding(&state.pool, test_required_group_name()).await;
    let mut tx = state.pool.begin().await.expect("begin tx");
    let target_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Confirmed OAuth",
            chosen_email: Some("manual@example.com".to_string()),
            verified_email: Some("old-verified@example.com".to_string()),
            group_name: Some(test_required_group_name().to_string()),
            is_mother: true,
            note: Some("keep this note".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "old-verified@example.com",
                Some("org_old"),
                Some("user_old"),
            ),
            encrypted_credentials: "encrypted-old".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert target oauth");
    tx.commit().await.expect("commit target");

    let relogin = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: None,
            email: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            account_id: Some(target_id),
            tag_ids: vec![],
            is_mother: None,
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create relogin session")
    .0;
    let session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load relogin session")
        .expect("relogin session should exist");
    let err = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            session,
            display_name: "Confirmed OAuth".to_string(),
            chosen_email: Some("manual@example.com".to_string()),
            verified_email: Some("new-verified@example.com".to_string()),
            claims: test_claims(
                "new-verified@example.com",
                Some("org_new"),
                Some("user_new"),
            ),
            encrypted_credentials: "encrypted-new".to_string(),
            has_refresh_token: false,
            token_expires_at: "2026-05-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect_err("different identity should require confirmation");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert_eq!(err.1, "OAuth identity confirmation required");

    let unchanged = load_upstream_account_row(&state.pool, target_id)
        .await
        .expect("load unchanged target")
        .expect("target exists");
    assert_eq!(
        unchanged.verified_email.as_deref(),
        Some("old-verified@example.com")
    );
    assert_eq!(unchanged.chatgpt_account_id.as_deref(), Some("org_old"));
    assert_eq!(unchanged.chatgpt_user_id.as_deref(), Some("user_old"));
    assert_eq!(
        unchanged.encrypted_credentials.as_deref(),
        Some("encrypted-old")
    );

    let pending = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load pending confirmation")
        .expect("pending confirmation exists");
    assert_eq!(
        pending.status,
        LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION
    );
    assert_eq!(
        pending.pending_verified_email.as_deref(),
        Some("new-verified@example.com")
    );
    assert_eq!(
        pending.pending_chatgpt_account_id.as_deref(),
        Some("org_new")
    );
    assert_eq!(pending.pending_chatgpt_user_id.as_deref(), Some("user_new"));
    assert_eq!(
        pending.pending_encrypted_credentials.as_deref(),
        Some("encrypted-new")
    );

    let confirmed_id = confirm_oauth_identity_overwrite_inner(state.as_ref(), &relogin.login_id)
        .await
        .expect("confirm identity overwrite");
    assert_eq!(confirmed_id, target_id);

    let confirmed = load_upstream_account_row(&state.pool, target_id)
        .await
        .expect("load confirmed target")
        .expect("target exists");
    assert_eq!(confirmed.display_name, "Confirmed OAuth");
    assert_eq!(confirmed.email.as_deref(), Some("manual@example.com"));
    assert_eq!(
        confirmed.group_name.as_deref(),
        Some(test_required_group_name())
    );
    assert_eq!(confirmed.note.as_deref(), Some("keep this note"));
    assert_eq!(confirmed.is_mother, 1);
    assert_eq!(
        confirmed.verified_email.as_deref(),
        Some("new-verified@example.com")
    );
    assert_eq!(confirmed.chatgpt_account_id.as_deref(), Some("org_new"));
    assert_eq!(confirmed.chatgpt_user_id.as_deref(), Some("user_new"));
    assert_eq!(
        confirmed.encrypted_credentials.as_deref(),
        Some("encrypted-new")
    );
    assert_eq!(confirmed.has_refresh_token, Some(0));
    let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load completed session")
        .expect("completed session exists");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert!(completed_session.pending_encrypted_credentials.is_none());
}

#[tokio::test]
async fn expired_identity_confirmation_rejects_confirm_and_clears_pending_credentials() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_test_group_binding(&state.pool, test_required_group_name()).await;
    let mut tx = state.pool.begin().await.expect("begin tx");
    let target_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Expired Confirmation OAuth",
            chosen_email: Some("expired-manual@example.com".to_string()),
            verified_email: Some("expired-old@example.com".to_string()),
            group_name: Some(test_required_group_name().to_string()),
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "expired-old@example.com",
                Some("expired_org_old"),
                Some("expired_user_old"),
            ),
            encrypted_credentials: "encrypted-expired-old".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert target oauth");
    tx.commit().await.expect("commit target");

    let relogin = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: None,
            email: None,
            group_name: None,
            group_bound_proxy_keys: None,
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            account_id: Some(target_id),
            tag_ids: vec![],
            is_mother: None,
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create relogin session")
    .0;
    let session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load relogin session")
        .expect("relogin session should exist");
    persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            session,
            display_name: "Expired Confirmation OAuth".to_string(),
            chosen_email: Some("expired-manual@example.com".to_string()),
            verified_email: Some("expired-new@example.com".to_string()),
            claims: test_claims(
                "expired-new@example.com",
                Some("expired_org_new"),
                Some("expired_user_new"),
            ),
            encrypted_credentials: "encrypted-expired-new".to_string(),
            has_refresh_token: false,
            token_expires_at: "2026-05-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect_err("different identity should require confirmation");

    sqlx::query("UPDATE pool_oauth_login_sessions SET expires_at = ?2 WHERE login_id = ?1")
        .bind(&relogin.login_id)
        .bind("2020-01-01T00:00:00Z")
        .execute(&state.pool)
        .await
        .expect("expire pending confirmation session");

    let mut headers = HeaderMap::new();
    headers.insert("host", "127.0.0.1:8080".parse().expect("host header"));
    headers.insert(
        "origin",
        "http://127.0.0.1:8080".parse().expect("origin header"),
    );
    let err = confirm_oauth_login_session_identity_overwrite(
        State(state.clone()),
        headers,
        AxumPath(relogin.login_id.clone()),
    )
    .await
    .expect_err("expired confirmation should not apply credentials");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);

    let expired_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load expired session")
        .expect("expired session should exist");
    assert_eq!(expired_session.status, LOGIN_SESSION_STATUS_EXPIRED);
    assert!(expired_session.pending_encrypted_credentials.is_none());
    assert!(expired_session.pending_token_expires_at.is_none());
    assert!(expired_session.pending_verified_email.is_none());
    assert!(expired_session.pending_chatgpt_account_id.is_none());
    assert!(expired_session.pending_chatgpt_user_id.is_none());
    assert!(expired_session.pending_plan_type.is_none());
    assert!(expired_session.pending_has_refresh_token.is_none());

    let unchanged = load_upstream_account_row(&state.pool, target_id)
        .await
        .expect("load unchanged target")
        .expect("target exists");
    assert_eq!(
        unchanged.encrypted_credentials.as_deref(),
        Some("encrypted-expired-old")
    );
    assert_eq!(
        unchanged.verified_email.as_deref(),
        Some("expired-old@example.com")
    );
    assert_eq!(
        unchanged.chatgpt_account_id.as_deref(),
        Some("expired_org_old")
    );
    assert_eq!(
        unchanged.chatgpt_user_id.as_deref(),
        Some("expired_user_old")
    );
}

#[tokio::test]
async fn upsert_oauth_account_preserves_route_cooldown_state_for_existing_account() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx");
    ensure_display_name_available(&mut *tx, "Cooldown OAuth Existing", None)
        .await
        .expect("name available");
    let account_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Cooldown OAuth Existing",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "cooldown-existing@example.com",
                Some("cooldown_org"),
                Some("cooldown_user"),
            ),
            encrypted_credentials: "encrypted-original".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert oauth account");
    tx.commit().await.expect("commit insert tx");

    seed_route_cooldown(
        &pool,
        account_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        300,
    )
    .await;
    let before = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row before update")
        .expect("row exists before update");

    let mut tx = pool.begin().await.expect("begin update tx");
    let updated_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(account_id),
            display_name: "Cooldown OAuth Existing",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: Some("updated note".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "cooldown-existing@example.com",
                Some("cooldown_org"),
                Some("cooldown_user"),
            ),
            encrypted_credentials: "encrypted-updated".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-15T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("update oauth account");
    tx.commit().await.expect("commit update tx");

    assert_eq!(updated_id, account_id);
    let after = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load row after update")
        .expect("row exists after update");

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
    assert_eq!(after.note.as_deref(), Some("updated note"));
    assert_eq!(
        after.encrypted_credentials.as_deref(),
        Some("encrypted-updated")
    );
}

#[tokio::test]
async fn update_account_preserves_system_tags_when_empty_tag_ids_are_sent() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Tag Preserve Target").await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");

    let original_tag_ids = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load original account tags");
    assert!(!original_tag_ids.is_empty());

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
                note: Some("preserved note".to_string()),
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
                tag_ids: Some(vec![]),
                routing_rule: None,
                ..UpdateUpstreamAccountRequest::default()
            },
        )
        .await
        .expect("update should preserve system tags");

    assert_eq!(detail.note.as_deref(), Some("preserved note"));

    let updated_tag_ids = sqlx::query_scalar::<_, i64>(
        r#"
            SELECT tag_id
            FROM pool_upstream_account_tags
            WHERE account_id = ?1
            ORDER BY tag_id ASC
            "#,
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load updated account tags");
    assert_eq!(updated_tag_ids, original_tag_ids);
}

#[tokio::test]
async fn refresh_rejects_display_name_renames_that_conflict_with_other_accounts() {
    let pool = test_pool().await;
    let crypto_key = derive_secret_key("refresh-display-name-conflict");
    let account_id = insert_syncable_oauth_account(
        &pool,
        &crypto_key,
        "old@example.com",
        "old@example.com",
        "refresh_conflict_org",
        "refresh_conflict_user",
    )
    .await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET verified_email = ?2
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("old@example.com")
    .execute(&pool)
    .await
    .expect("seed verified email");
    let conflicting_id = insert_api_key_account(&pool, "renamed@example.com").await;

    let err = persist_oauth_credentials(
        &pool,
        account_id,
        &crypto_key,
        &StoredOauthCredentials {
            access_token: "access-2".to_string(),
            refresh_token: Some("refresh-2".to_string()),
            id_token: test_id_token(
                "renamed@example.com",
                Some("refresh_conflict_org"),
                Some("refresh_conflict_user"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        },
        "2026-03-16T00:00:00Z",
    )
    .await
    .expect_err("reject conflicting refresh rename");
    assert!(
        err.to_string().contains("displayName must be unique"),
        "unexpected error: {err:#}"
    );

    let row = load_upstream_account_row(&pool, account_id)
        .await
        .expect("load oauth row")
        .expect("oauth row exists");
    assert_eq!(row.display_name, "old@example.com");
    assert_eq!(row.email.as_deref(), Some("old@example.com"));
    assert_eq!(row.verified_email.as_deref(), Some("old@example.com"));

    let conflict = load_upstream_account_row(&pool, conflicting_id)
        .await
        .expect("load conflicting row")
        .expect("conflicting row exists");
    assert_eq!(conflict.display_name, "renamed@example.com");
}

#[tokio::test]
async fn relink_updates_existing_oauth_row_without_inserting() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx");
    let original_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Original OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: Some("prod".to_string()),
            is_mother: false,
            note: Some("note".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims("first@example.com", Some("org_shared"), Some("user_1")),
            encrypted_credentials: "encrypted-1".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert original oauth");
    tx.commit().await.expect("commit tx");

    let mut tx = pool.begin().await.expect("begin relink tx");
    ensure_display_name_available(&mut *tx, "Renamed OAuth", Some(original_id))
        .await
        .expect("name available");
    let relinked_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: Some(original_id),
            display_name: "Renamed OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: Some("prod".to_string()),
            is_mother: false,
            note: Some("fresh".to_string()),
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims("second@example.com", Some("org_shared"), Some("user_9")),
            encrypted_credentials: "encrypted-2".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-15T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("relink oauth");
    tx.commit().await.expect("commit relink tx");

    assert_eq!(relinked_id, original_id);
    let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_accounts")
        .fetch_one(&pool)
        .await
        .expect("count accounts");
    assert_eq!(count, 1);

    let renamed = load_upstream_account_row(&pool, original_id)
        .await
        .expect("load updated row")
        .expect("row exists");
    assert_eq!(renamed.display_name, "Renamed OAuth");
    assert_eq!(renamed.chatgpt_user_id.as_deref(), Some("user_9"));
}

#[tokio::test]
async fn display_name_uniqueness_is_case_insensitive_and_self_excluding() {
    let pool = test_pool().await;
    let account_id = insert_api_key_account(&pool, " Alpha ").await;

    let mut tx = pool.begin().await.expect("begin tx conflict");
    let conflict = ensure_display_name_available(&mut *tx, "alpha", None).await;
    assert_eq!(
        conflict,
        Err((
            StatusCode::CONFLICT,
            "displayName must be unique".to_string()
        ))
    );

    let allowed = ensure_display_name_available(&mut *tx, " alpha ", Some(account_id)).await;
    assert!(allowed.is_ok());
}
