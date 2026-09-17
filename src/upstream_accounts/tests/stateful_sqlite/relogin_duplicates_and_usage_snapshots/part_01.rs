use super::*;
use axum::http::header;

async fn seed_latest_usage_plan_accounts(pool: &SqlitePool) -> Vec<i64> {
    let mut inserted_ids = Vec::new();
    for (display_name, email, plan_type) in [
        ("Legacy Team One", "legacy-team-1@example.com", None),
        ("Legacy Team Two", "legacy-team-2@example.com", Some("pro")),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("legacy_shared_org"),
                    None,
                    plan_type,
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }
    inserted_ids
}

async fn assert_no_duplicate_details(pool: &SqlitePool, account_ids: &[i64]) {
    for account_id in account_ids {
        let detail = load_upstream_account_detail(pool, *account_id)
            .await
            .expect("load account detail")
            .expect("account detail exists");
        assert!(detail.summary.duplicate_info.is_none());
    }
}

async fn assert_completed_relogin_rejected(
    state: &Arc<AppState>,
    login_id: &str,
    base_updated_at: &str,
) {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(base_updated_at).expect("valid updated_at header"),
    );
    let err = update_oauth_login_session(
        State(state.clone()),
        headers,
        AxumPath(login_id.to_string()),
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
}

async fn seed_shared_identity_relogin_target(pool: &SqlitePool) -> i64 {
    ensure_test_group_binding(pool, test_required_group_name()).await;
    let mut tx = pool.begin().await.expect("begin tx");
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
    target_id
}

async fn seed_mixed_plan_same_name_accounts(
    state: &Arc<AppState>,
    display_name: &str,
    shared_account_id: &str,
) -> Vec<i64> {
    let mut inserted_ids = Vec::new();
    for (email, plan_type) in [
        ("shared-mixed-team@example.com", Some("team")),
        ("shared-mixed-pro@example.com", Some("pro")),
    ] {
        let mut tx = state.pool.begin().await.expect("begin tx");
        ensure_display_name_available_for_oauth_identity(
            &mut *tx,
            display_name,
            None,
            Some(shared_account_id),
            None,
            None,
            plan_type,
        )
        .await
        .expect("mixed-plan same-name create should be allowed");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some(shared_account_id),
                    None,
                    plan_type,
                ),
                encrypted_credentials: format!("encrypted-{email}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }
    inserted_ids
}

async fn seed_confirmed_oauth_relogin_target(pool: &SqlitePool) -> i64 {
    ensure_test_group_binding(pool, test_required_group_name()).await;
    let mut tx = pool.begin().await.expect("begin tx");
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
    target_id
}

async fn assert_pending_identity_confirmation(state: &AppState, login_id: &str) {
    let pending = load_login_session_by_login_id(&state.pool, login_id)
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
}

async fn assert_original_identity_unchanged(state: &AppState, account_id: i64) {
    let unchanged = load_upstream_account_row(&state.pool, account_id)
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
}

async fn expire_and_reject_identity_confirmation(state: &Arc<AppState>, login_id: &str) {
    sqlx::query("UPDATE pool_oauth_login_sessions SET expires_at = ?2 WHERE login_id = ?1")
        .bind(login_id)
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
        AxumPath(login_id.to_string()),
    )
    .await
    .expect_err("expired confirmation should not apply credentials");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

async fn assert_expired_confirmation_cleared(state: &AppState, login_id: &str, account_id: i64) {
    let expired_session = load_login_session_by_login_id(&state.pool, login_id)
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
    let unchanged = load_upstream_account_row(&state.pool, account_id)
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
pub(crate) async fn update_oauth_login_session_rejects_completed_relogin_repairs() {
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

    let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load completed relogin session")
        .expect("completed relogin session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(
        completed_session.updated_at,
        completed_session.consumed_at.clone().unwrap()
    );

    assert_completed_relogin_rejected(&state, &relogin.login_id, &relogin.updated_at).await;

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load relogin target after rejected repair")
        .expect("relogin target should exist");
    assert_eq!(account.display_name, "Relogin Target");
    assert_ne!(account.group_name.as_deref(), Some("edited-group"));
    assert_ne!(account.note.as_deref(), Some("edited note"));
}

#[tokio::test]
pub(crate) async fn relogin_same_identity_skips_sibling_display_name_conflict() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let target_id = seed_shared_identity_relogin_target(&state.pool).await;

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
pub(crate) async fn relogin_different_identity_requires_confirmation_then_preserves_display_fields()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let target_id = seed_confirmed_oauth_relogin_target(&state.pool).await;

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

    assert_original_identity_unchanged(state.as_ref(), target_id).await;

    assert_pending_identity_confirmation(state.as_ref(), &relogin.login_id).await;

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
pub(crate) async fn expired_identity_confirmation_rejects_confirm_and_clears_pending_credentials() {
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

    expire_and_reject_identity_confirmation(&state, &relogin.login_id).await;
    assert_expired_confirmation_cleared(state.as_ref(), &relogin.login_id, target_id).await;
}

#[tokio::test]
pub(crate) async fn upsert_oauth_account_preserves_route_cooldown_state_for_existing_account() {
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
pub(crate) async fn distinct_members_with_shared_account_id_are_not_flagged_as_duplicates() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx 1");
    ensure_display_name_available(&mut *tx, "First OAuth", None)
        .await
        .expect("first name available");
    let first_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "First OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "first@example.com",
                Some("org_shared"),
                Some("user_1"),
                Some("k12"),
            ),
            encrypted_credentials: "encrypted-1".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("first oauth insert");
    tx.commit().await.expect("commit tx 1");

    let mut tx = pool.begin().await.expect("begin tx 2");
    ensure_display_name_available(&mut *tx, "Second OAuth", None)
        .await
        .expect("second name available");
    let second_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Second OAuth",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims_with_plan_type(
                "second@example.com",
                Some("org_shared"),
                Some("user_2"),
                Some("k12"),
            ),
            encrypted_credentials: "encrypted-2".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("second oauth insert");
    tx.commit().await.expect("commit tx 2");

    assert_ne!(first_id, second_id);
    let count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_upstream_accounts WHERE kind = ?1")
            .bind(UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX)
            .fetch_one(&pool)
            .await
            .expect("count oauth rows");
    assert_eq!(count, 2);

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(!duplicate_info.contains_key(&first_id));
    assert!(!duplicate_info.contains_key(&second_id));

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| summary.id == first_id || summary.id == second_id)
            .all(|summary| summary.duplicate_info.is_none())
    );

    assert_no_duplicate_details(&pool, &[first_id, second_id]).await;
}

#[tokio::test]
pub(crate) async fn load_duplicate_info_for_account_matches_global_duplicate_result() {
    let pool = test_pool().await;

    let mut tx = pool.begin().await.expect("begin tx 1");
    ensure_display_name_available(&mut *tx, "Scoped Duplicate One", None)
        .await
        .expect("first name available");
    let first_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Scoped Duplicate One",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "scoped-1@example.com",
                Some("scoped_shared_org"),
                Some("scoped_user_shared"),
            ),
            encrypted_credentials: "encrypted-scoped-1".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("first oauth insert");
    tx.commit().await.expect("commit tx 1");

    let mut tx = pool.begin().await.expect("begin tx 2");
    ensure_display_name_available(&mut *tx, "Scoped Duplicate Two", None)
        .await
        .expect("second name available");
    let second_id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: "Scoped Duplicate Two",
            chosen_email: None,
            verified_email: None,
            group_name: None,
            is_mother: false,
            note: None,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                "scoped-2@example.com",
                Some("scoped_shared_org"),
                Some("scoped_user_shared"),
            ),
            encrypted_credentials: "encrypted-scoped-2".to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("second oauth insert");
    tx.commit().await.expect("commit tx 2");

    let all_duplicates = load_duplicate_info_map(&pool)
        .await
        .expect("load all duplicate info");
    let first_scoped = load_duplicate_info_for_account(&pool, first_id)
        .await
        .expect("load scoped duplicate for first");
    let second_scoped = load_duplicate_info_for_account(&pool, second_id)
        .await
        .expect("load scoped duplicate for second");

    let first_all = all_duplicates
        .get(&first_id)
        .expect("first global duplicate");
    let first_scoped = first_scoped.expect("first scoped duplicate");
    assert_eq!(first_scoped.peer_account_ids, first_all.peer_account_ids);
    assert_eq!(first_scoped.reasons, first_all.reasons);

    let second_all = all_duplicates
        .get(&second_id)
        .expect("second global duplicate");
    let second_scoped = second_scoped.expect("second scoped duplicate");
    assert_eq!(second_scoped.peer_account_ids, second_all.peer_account_ids);
    assert_eq!(second_scoped.reasons, second_all.reasons);
}

#[tokio::test]
pub(crate) async fn same_group_team_shared_org_accounts_are_not_flagged_as_duplicates() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email, user_id) in [
        ("Team Member One", "team-1@example.com", "team_user_1"),
        ("Team Member Two", "team-2@example.com", "team_user_2"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: Some("0414-3".to_string()),
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("shared_team_org"),
                    Some(user_id),
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(duplicate_info.is_empty());

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| summary.duplicate_info.is_none() && !summary.is_mother)
    );

    for account_id in inserted_ids {
        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(detail.summary.duplicate_info.is_none());
        assert!(!detail.summary.is_mother);
    }
}

#[tokio::test]
pub(crate) async fn same_group_team_shared_org_accounts_keep_manual_mother_only() {
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email, user_id, is_mother) in [
        (
            "Manual Mother One",
            "manual-mother-1@example.com",
            "manual_user_1",
            false,
        ),
        (
            "Manual Mother Two",
            "manual-mother-2@example.com",
            "manual_user_2",
            true,
        ),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: Some("shared-team-manual".to_string()),
                is_mother,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("shared_team_org_manual"),
                    Some(user_id),
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    let mother_ids = summaries
        .iter()
        .filter(|summary| inserted_ids.contains(&summary.id) && summary.is_mother)
        .map(|summary| summary.id)
        .collect::<Vec<_>>();
    assert_eq!(mother_ids, vec![inserted_ids[1]]);

    let first_detail = load_upstream_account_detail(&pool, inserted_ids[0])
        .await
        .expect("load first detail")
        .expect("first detail exists");
    let second_detail = load_upstream_account_detail(&pool, inserted_ids[1])
        .await
        .expect("load second detail")
        .expect("second detail exists");
    assert!(!first_detail.summary.is_mother);
    assert!(second_detail.summary.is_mother);
    assert!(first_detail.summary.duplicate_info.is_none());
    assert!(second_detail.summary.duplicate_info.is_none());
}

#[tokio::test]
pub(crate) async fn new_oauth_accounts_with_shared_user_id_are_preserved_and_flagged() {
    let pool = test_pool().await;

    for (display_name, email, account_id) in [
        ("First OAuth", "first@example.com", "org_1"),
        ("Second OAuth", "second@example.com", "org_2"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims(email, Some(account_id), Some("user_shared")),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptUserId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(summaries.iter().all(|summary| matches!(
                    summary.duplicate_info.as_ref().map(|info| info.reasons.as_slice()),
                    Some([DuplicateReason::SharedChatgptUserId])
                )));

    for summary in summaries {
        let detail = load_upstream_account_detail(&pool, summary.id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptUserId])
        ));
    }
}

#[tokio::test]
pub(crate) async fn legacy_shared_account_id_duplicates_ignore_mixed_plan_types() {
    let pool = test_pool().await;

    for (display_name, email, plan_type) in [
        ("Team OAuth", "team@example.com", Some("team")),
        ("Personal OAuth", "personal@example.com", Some("pro")),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(email, Some("org_shared"), None, plan_type),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(summaries.iter().all(|summary| matches!(
                summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptAccountId])
            )));

    for summary in summaries {
        let detail = load_upstream_account_detail(&pool, summary.id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
    }
}

#[tokio::test]
pub(crate) async fn shared_user_id_duplicates_ignore_mixed_plan_types() {
    let pool = test_pool().await;

    for (display_name, email, account_id, plan_type) in [
        ("Team OAuth", "team@example.com", "org_team", Some("team")),
        (
            "Personal OAuth",
            "personal@example.com",
            "org_personal",
            Some("free"),
        ),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some(account_id),
                    Some("user_shared"),
                    plan_type,
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptUserId])
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(summaries.iter().all(|summary| matches!(
                summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptUserId])
            )));

    for summary in summaries {
        let detail = load_upstream_account_detail(&pool, summary.id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptUserId])
        ));
    }
}

#[tokio::test]
pub(crate) async fn latest_usage_sample_plan_type_restores_same_plan_duplicate_flags() {
    let pool = test_pool().await;

    let inserted_ids = seed_latest_usage_plan_accounts(&pool).await;

    for (index, account_id) in inserted_ids.iter().enumerate() {
        insert_limit_sample(
            &pool,
            *account_id,
            &format!("2026-03-15T00:00:0{}Z", index + 1),
            Some("team"),
        )
        .await;
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type_observed_at = '2026-03-14T00:00:00Z',
                    last_refreshed_at = '2026-03-14T00:00:00Z',
                    updated_at = '2026-03-14T00:00:00Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("age account claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert_eq!(duplicate_info.len(), 2);
    assert!(
        duplicate_info
            .values()
            .all(|info| { info.reasons == vec![DuplicateReason::SharedChatgptAccountId] })
    );

    let summaries = load_upstream_account_summaries(
        &pool,
        &usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test"),
    )
    .await
    .expect("load summaries");
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| summary.plan_type.as_deref() == Some("team"))
    );
    assert!(
        summaries
            .iter()
            .filter(|summary| inserted_ids.contains(&summary.id))
            .all(|summary| matches!(
                summary
                    .duplicate_info
                    .as_ref()
                    .map(|info| info.reasons.as_slice()),
                Some([DuplicateReason::SharedChatgptAccountId])
            ))
    );

    for account_id in inserted_ids {
        let detail = load_upstream_account_detail(&pool, account_id)
            .await
            .expect("load detail")
            .expect("detail exists");
        assert_eq!(detail.summary.plan_type.as_deref(), Some("team"));
        assert!(matches!(
            detail
                .summary
                .duplicate_info
                .as_ref()
                .map(|info| info.reasons.as_slice()),
            Some([DuplicateReason::SharedChatgptAccountId])
        ));
    }
}

#[tokio::test]
pub(crate) async fn latest_usage_sample_plan_type_does_not_clear_legacy_shared_account_duplicates()
{
    let pool = test_pool().await;

    let mut inserted_ids = Vec::new();
    for (display_name, email) in [
        ("Stale Team One", "stale-team-1@example.com"),
        ("Stale Team Two", "stale-team-2@example.com"),
    ] {
        let mut tx = pool.begin().await.expect("begin tx");
        ensure_display_name_available(&mut *tx, display_name, None)
            .await
            .expect("name available");
        let account_id = upsert_oauth_account(
            &mut tx,
            OauthAccountUpsert {
                account_id: None,
                display_name,
                chosen_email: None,
                verified_email: None,
                group_name: None,
                is_mother: false,
                note: None,
                tag_ids: vec![],
                requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
                claims: &test_claims_with_plan_type(
                    email,
                    Some("stale_shared_org"),
                    None,
                    Some("team"),
                ),
                encrypted_credentials: format!("encrypted-{display_name}"),
                has_refresh_token: true,
                token_expires_at: "2026-03-14T00:00:00Z",
                external_identity: None,
            },
        )
        .await
        .expect("oauth insert");
        tx.commit().await.expect("commit tx");
        inserted_ids.push(account_id);
    }

    insert_limit_sample(&pool, inserted_ids[0], "2026-03-15T00:00:01Z", Some("team")).await;
    insert_limit_sample(&pool, inserted_ids[1], "2026-03-15T00:00:02Z", Some("pro")).await;
    for account_id in &inserted_ids {
        sqlx::query(
            r#"
                UPDATE pool_upstream_accounts
                SET plan_type_observed_at = '2026-03-14T00:00:00Z',
                    last_refreshed_at = '2026-03-14T00:00:00Z',
                    updated_at = '2026-03-14T00:00:00Z'
                WHERE id = ?1
                "#,
        )
        .bind(*account_id)
        .execute(&pool)
        .await
        .expect("age account claims");
    }

    let duplicate_info = load_duplicate_info_map(&pool)
        .await
        .expect("load duplicate info");
    assert!(
        duplicate_info
            .values()
            .all(|value| value.reasons == vec![DuplicateReason::SharedChatgptAccountId])
    );
}

#[tokio::test]
pub(crate) async fn update_uses_latest_sample_plan_type_for_current_mixed_plan_same_name_exemption()
{
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let display_name = "Shared Mixed Plan";
    let shared_account_id = "shared_mixed_plan_account";
    let inserted_ids =
        seed_mixed_plan_same_name_accounts(&state, display_name, shared_account_id).await;

    let current_account_id = inserted_ids[1];
    insert_limit_sample(
        &state.pool,
        current_account_id,
        "2026-03-15T00:00:02Z",
        Some("pro"),
    )
    .await;
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET plan_type = NULL,
                plan_type_observed_at = NULL,
                updated_at = '2026-03-15T00:00:03Z'
            WHERE id = ?1
            "#,
    )
    .bind(current_account_id)
    .execute(&state.pool)
    .await
    .expect("clear current account plan type");

    let detail = state
        .upstream_accounts
        .account_ops
        .run_update_account(
            state.clone(),
            current_account_id,
            UpdateUpstreamAccountRequest {
                display_name: None,
                email: OptionalField::Missing,
                group_name: None,
                group_bound_proxy_keys: None,
                group_node_shunt_enabled: None,
                group_single_account_rotation_enabled: None,
                note: Some("mixed-plan note update".to_string()),
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
            },
        )
        .await
        .expect("update should keep mixed-plan same-name exemption");

    assert_eq!(detail.summary.display_name, display_name);
    assert_eq!(detail.note.as_deref(), Some("mixed-plan note update"));
}
