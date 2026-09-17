async fn seed_completed_relogin_target(state: &Arc<AppState>) -> (LoginSessionStatusResponse, i64) {
    let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
    insert_oauth_account(&state.pool, "Edited Relogin").await;
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
    let pending = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load relogin session")
        .expect("relogin session should exist");
    let key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let credentials = encrypt_credentials(
        key,
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
    let completed_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: "Edited Relogin".to_string(),
            chosen_email: None,
            verified_email: None,
            session: pending,
            claims: test_claims("relogin@example.com", Some("org_test"), Some("user_test")),
            encrypted_credentials: credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist relogin callback");
    assert_eq!(completed_id, account_id);
    (relogin, account_id)
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_completed_relogin_repairs() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (relogin, account_id) = seed_completed_relogin_target(&state).await;

    let completed_session = load_login_session_by_login_id(&state.pool, &relogin.login_id)
        .await
        .expect("load completed relogin session")
        .expect("completed relogin session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(
        completed_session.updated_at,
        completed_session.consumed_at.clone().unwrap()
    );

    assert_completed_relogin_repair_rejected(
        state,
        &relogin.login_id,
        account_id,
        &relogin.updated_at,
    )
    .await;
}

#[tokio::test]
pub(crate) async fn relogin_same_identity_skips_sibling_display_name_conflict() {
    relogin_same_identity_skips_sibling_display_name_conflict_impl().await;
}

async fn relogin_same_identity_skips_sibling_display_name_conflict_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let target_id = seed_same_identity_relogin_accounts(&state).await;

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

    assert_same_identity_relogin_result(&state.pool, target_id, &relogin.login_id).await;
}

async fn seed_same_identity_relogin_accounts(state: &Arc<AppState>) -> i64 {
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
    target_id
}

async fn assert_same_identity_relogin_result(pool: &SqlitePool, target_id: i64, login_id: &str) {
    let account = load_upstream_account_row(pool, target_id)
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
    let completed_session = load_login_session_by_login_id(pool, login_id)
        .await
        .expect("load completed session")
        .expect("completed session exists");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
}

async fn assert_completed_relogin_repair_rejected(
    state: Arc<AppState>,
    login_id: &str,
    account_id: i64,
    base_updated_at: &str,
) {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(base_updated_at).expect("valid updated_at header"),
    );
    let error = update_oauth_login_session(
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
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        error.1,
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
pub(crate) async fn relogin_different_identity_requires_confirmation_then_preserves_display_fields()
{
    relogin_different_identity_requires_confirmation_then_preserves_display_fields_impl().await;
}

async fn relogin_different_identity_requires_confirmation_then_preserves_display_fields_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_test_group_binding(&state.pool, test_required_group_name()).await;
    let target_id = insert_relogin_target(
        &state.pool,
        ReloginTargetFixture {
            display_name: "Confirmed OAuth",
            chosen_email: "manual@example.com",
            verified_email: "old-verified@example.com",
            group_name: test_required_group_name(),
            is_mother: true,
            note: Some("keep this note"),
            email_claim: "old-verified@example.com",
            account_claim: "org_old",
            user_claim: "user_old",
            encrypted_credentials: "encrypted-old",
        },
    )
    .await;

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

    assert_identity_confirmation_pending(&state.pool, &relogin.login_id, target_id).await;

    let confirmed_id = confirm_oauth_identity_overwrite_inner(state.as_ref(), &relogin.login_id)
        .await
        .expect("confirm identity overwrite");
    assert_eq!(confirmed_id, target_id);
    assert_confirmed_identity_overwrite(&state.pool, &relogin.login_id, target_id).await;
}

async fn assert_identity_confirmation_pending(pool: &SqlitePool, login_id: &str, target_id: i64) {
    let unchanged = load_upstream_account_row(pool, target_id)
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
    let pending = load_login_session_by_login_id(pool, login_id)
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

async fn assert_confirmed_identity_overwrite(pool: &SqlitePool, login_id: &str, target_id: i64) {
    let confirmed = load_upstream_account_row(pool, target_id)
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
    let completed_session = load_login_session_by_login_id(pool, login_id)
        .await
        .expect("load completed session")
        .expect("completed session exists");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert!(completed_session.pending_encrypted_credentials.is_none());
}

#[tokio::test]
pub(crate) async fn expired_identity_confirmation_rejects_confirm_and_clears_pending_credentials() {
    expired_identity_confirmation_rejects_confirm_and_clears_pending_credentials_impl().await;
}

async fn expired_identity_confirmation_rejects_confirm_and_clears_pending_credentials_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    ensure_test_group_binding(&state.pool, test_required_group_name()).await;
    let target_id = insert_relogin_target(
        &state.pool,
        ReloginTargetFixture {
            display_name: "Expired Confirmation OAuth",
            chosen_email: "expired-manual@example.com",
            verified_email: "expired-old@example.com",
            group_name: test_required_group_name(),
            is_mother: false,
            note: None,
            email_claim: "expired-old@example.com",
            account_claim: "expired_org_old",
            user_claim: "expired_user_old",
            encrypted_credentials: "encrypted-expired-old",
        },
    )
    .await;

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

    assert_expired_confirmation_cleared(&state.pool, &relogin.login_id, target_id).await;
}

async fn assert_expired_confirmation_cleared(pool: &SqlitePool, login_id: &str, target_id: i64) {
    let expired_session = load_login_session_by_login_id(pool, login_id)
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
    let unchanged = load_upstream_account_row(pool, target_id)
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
    let first_id =
        insert_shared_account_member(&pool, "First OAuth", "first@example.com", "user_1").await;
    let second_id =
        insert_shared_account_member(&pool, "Second OAuth", "second@example.com", "user_2").await;

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

    let first_detail = load_upstream_account_detail(&pool, first_id)
        .await
        .expect("load first detail")
        .expect("first detail exists");
    let second_detail = load_upstream_account_detail(&pool, second_id)
        .await
        .expect("load second detail")
        .expect("second detail exists");
    assert!(first_detail.summary.duplicate_info.is_none());
    assert!(second_detail.summary.duplicate_info.is_none());
}

async fn insert_shared_account_member(
    pool: &SqlitePool,
    display_name: &str,
    email: &str,
    user_id: &str,
) -> i64 {
    let mut tx = pool.begin().await.expect("begin shared-account member tx");
    ensure_display_name_available(&mut *tx, display_name, None)
        .await
        .expect("shared-account member name available");
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
                Some("org_shared"),
                Some(user_id),
                Some("k12"),
            ),
            encrypted_credentials: format!("encrypted-{user_id}"),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("shared-account member insert");
    tx.commit().await.expect("commit shared-account member tx");
    account_id
}

struct ReloginTargetFixture<'a> {
    display_name: &'a str,
    chosen_email: &'a str,
    verified_email: &'a str,
    group_name: &'a str,
    is_mother: bool,
    note: Option<&'a str>,
    email_claim: &'a str,
    account_claim: &'a str,
    user_claim: &'a str,
    encrypted_credentials: &'a str,
}

async fn insert_relogin_target(pool: &SqlitePool, fixture: ReloginTargetFixture<'_>) -> i64 {
    let mut tx = pool.begin().await.expect("begin relogin target tx");
    let fixture_group = Some(fixture.group_name.to_string());
    let fixture_note = fixture.note.map(str::to_string);
    let id = upsert_oauth_account(
        &mut tx,
        OauthAccountUpsert {
            account_id: None,
            display_name: fixture.display_name,
            chosen_email: Some(fixture.chosen_email.to_string()),
            verified_email: Some(fixture.verified_email.to_string()),
            group_name: fixture_group,
            is_mother: fixture.is_mother,
            note: fixture_note,
            tag_ids: vec![],
            requested_group_metadata_changes: RequestedGroupMetadataChanges::default(),
            claims: &test_claims(
                fixture.email_claim,
                Some(fixture.account_claim),
                Some(fixture.user_claim),
            ),
            encrypted_credentials: fixture.encrypted_credentials.to_string(),
            has_refresh_token: true,
            token_expires_at: "2026-03-14T00:00:00Z",
            external_identity: None,
        },
    )
    .await
    .expect("insert relogin target");
    tx.commit().await.expect("commit relogin target");
    id
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

use super::*;
