use super::super::*;

#[tokio::test]
async fn update_oauth_login_session_preserves_pending_url_and_persists_metadata() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-1",
        "pending-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Original Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before".to_string()),
            group_note: Some("alpha note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Updated Pending".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("beta".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after".to_string()),
            group_note: OptionalField::Value("beta shared".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Value("mailbox-session-1".to_string()),
            mailbox_address: OptionalField::Value("pending-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("update oauth login session")
    .0;

    assert_eq!(updated.login_id, created.login_id);
    assert_eq!(updated.auth_url, created.auth_url);
    assert_eq!(updated.redirect_uri, created.redirect_uri);
    assert_eq!(updated.expires_at, created.expires_at);

    let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.display_name.as_deref(), Some("Updated Pending"));
    assert_eq!(stored.group_name.as_deref(), Some("beta"));
    assert_eq!(stored.note.as_deref(), Some("after"));
    assert_eq!(stored.group_note.as_deref(), Some("beta shared"));
    assert_eq!(stored.is_mother, 1);
    assert_eq!(
        parse_tag_ids_json(stored.tag_ids_json.as_deref()),
        Vec::<i64>::new()
    );
    assert_eq!(
        stored.mailbox_session_id.as_deref(),
        Some("mailbox-session-1")
    );
    assert_eq!(
        stored.mailbox_address.as_deref(),
        Some("pending-sync@mail-tw.707079.xyz")
    );
}

#[tokio::test]
async fn update_oauth_login_session_ignores_stale_baseline_updates() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Ordered Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before".to_string()),
            group_note: Some("alpha note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let mut newer_headers = HeaderMap::new();
    newer_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let newer = update_oauth_login_session(
        State(state.clone()),
        newer_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Newest Pending".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("beta".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("newest note".to_string()),
            group_note: OptionalField::Value("beta note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("apply newer oauth login session update")
    .0;
    assert_ne!(newer.updated_at, created.updated_at);
    let newer_updated_at = newer.updated_at.clone();

    let mut stale_headers = HeaderMap::new();
    stale_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let stale = update_oauth_login_session(
        State(state.clone()),
        stale_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Stale Pending".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("gamma".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("stale note".to_string()),
            group_note: OptionalField::Value("gamma note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("stale oauth login session update should be ignored")
    .0;

    assert_eq!(stale.login_id, created.login_id);
    assert_eq!(stale.updated_at, newer_updated_at);

    let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.display_name.as_deref(), Some("Newest Pending"));
    assert_eq!(stored.group_name.as_deref(), Some("beta"));
    assert_eq!(stored.note.as_deref(), Some("newest note"));
    assert_eq!(stored.group_note.as_deref(), Some("beta note"));
    assert_eq!(stored.is_mother, 1);
    assert_eq!(stored.updated_at, newer_updated_at);
}

#[tokio::test]
async fn update_oauth_login_session_preserves_omitted_fields() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-partial",
        "partial-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Keep Me".to_string()),
            email: None,
            group_name: Some("partial-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before partial patch".to_string()),
            group_note: Some("partial draft note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(true),
            mailbox_session_id: Some("mailbox-session-partial".to_string()),
            mailbox_address: Some("partial-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after partial patch".to_string()),
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Missing,
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("update oauth login session")
    .0;

    assert_eq!(updated.login_id, created.login_id);
    assert_eq!(updated.auth_url, created.auth_url);
    assert_eq!(updated.redirect_uri, created.redirect_uri);
    assert_eq!(updated.expires_at, created.expires_at);

    let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.display_name.as_deref(), Some("Keep Me"));
    assert_eq!(stored.group_name.as_deref(), Some("partial-group"));
    assert_eq!(stored.note.as_deref(), Some("after partial patch"));
    assert_eq!(stored.group_note.as_deref(), Some("partial draft note"));
    assert_eq!(stored.is_mother, 1);
    assert_eq!(
        parse_tag_ids_json(stored.tag_ids_json.as_deref()),
        Vec::<i64>::new()
    );
    assert_eq!(
        stored.mailbox_session_id.as_deref(),
        Some("mailbox-session-partial")
    );
    assert_eq!(
        stored.mailbox_address.as_deref(),
        Some("partial-sync@mail-tw.707079.xyz")
    );
}

#[tokio::test]
async fn update_oauth_login_session_clears_omitted_group_note_when_group_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Move Draft Group".to_string()),
            email: None,
            group_name: Some("before-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before draft note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Value("after-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Missing,
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("update oauth login session")
    .0;

    assert_eq!(updated.login_id, created.login_id);
    assert_eq!(updated.auth_url, created.auth_url);
    assert_eq!(updated.redirect_uri, created.redirect_uri);
    assert_eq!(updated.expires_at, created.expires_at);

    let stored = load_login_session_by_login_id(&state.pool, &updated.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(stored.group_name.as_deref(), Some("after-group"));
    assert_eq!(stored.group_note, None);
    assert_eq!(stored.note.as_deref(), Some("before note"));
}

#[tokio::test]
async fn update_oauth_login_session_normalizes_blank_group_to_default_group() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Clear Group Note".to_string()),
            email: None,
            group_name: Some("draft-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before clearing group".to_string()),
            group_note: Some("draft group note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let _updated = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Value(String::new()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Missing,
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("blank group should normalize to default group")
    .0;

    let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load stored login session")
        .expect("stored login session should exist");
    assert_eq!(
        stored.group_name.as_deref(),
        Some(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME)
    );
    assert_eq!(stored.group_note, None);
    assert_eq!(stored.note.as_deref(), Some("before clearing group"));
}

#[tokio::test]
async fn updated_oauth_login_session_metadata_is_used_when_callback_persists_account() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    insert_test_oauth_mailbox_session(
        &state.pool,
        "mailbox-session-2",
        "callback-sync@mail-tw.707079.xyz",
        OAUTH_MAILBOX_SOURCE_ATTACHED,
    )
    .await;

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Before Patch".to_string()),
            email: None,
            group_name: Some("old-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("old group note".to_string()),
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    let _ = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("After Patch".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("new-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after note".to_string()),
            group_note: OptionalField::Value("draft group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Value("mailbox-session-2".to_string()),
            mailbox_address: OptionalField::Value("callback-sync@mail-tw.707079.xyz".to_string()),
        }),
    )
    .await
    .expect("update oauth login session");

    let updated_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load updated session")
        .expect("updated session should exist");
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "callback-access".to_string(),
            refresh_token: Some("callback-refresh".to_string()),
            id_token: test_id_token(
                "callback@example.com",
                Some("org_callback"),
                Some("user_callback"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: updated_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: updated_session.clone(),
            claims: test_claims(
                "callback@example.com",
                Some("org_callback"),
                Some("user_callback"),
            ),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth account row")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "After Patch");
    assert_eq!(account.group_name.as_deref(), Some("new-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);

    let account_tag_ids = sqlx::query_scalar::<_, i64>(
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
    .expect("load oauth account tags");
    assert!(account_tag_ids.is_empty());

    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("new-group")
    .fetch_one(&state.pool)
    .await
    .expect("load group note");
    assert_eq!(group_note.as_deref(), Some("draft group note"));

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(completed_session.account_id, Some(account_id));
}

#[tokio::test]
async fn persist_oauth_callback_preserves_group_node_shunt_for_legacy_pending_session() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let bound_proxy_keys = test_required_group_bound_proxy_keys();
    let mut conn = state
        .pool
        .acquire()
        .await
        .expect("acquire group metadata conn");
    save_group_metadata_record_conn(
        &mut conn,
        "legacy-group",
        UpstreamAccountGroupMetadata {
            bound_proxy_keys: bound_proxy_keys.clone(),
            node_shunt_enabled: true,
            ..UpstreamAccountGroupMetadata::default()
        },
    )
    .await
    .expect("seed legacy group metadata");
    drop(conn);

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Legacy Pending".to_string()),
            email: None,
            group_name: Some("legacy-group".to_string()),
            group_bound_proxy_keys: Some(bound_proxy_keys.clone()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("legacy note".to_string()),
            group_note: None,
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect("create oauth login session")
    .0;

    sqlx::query(
        r#"
            UPDATE pool_oauth_login_sessions
            SET group_node_shunt_enabled = 0,
                group_node_shunt_enabled_requested = 0
            WHERE login_id = ?1
            "#,
    )
    .bind(&created.login_id)
    .execute(&state.pool)
    .await
    .expect("downgrade pending session to legacy node shunt fields");

    let pending_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load pending session")
        .expect("pending session should exist");
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted_credentials = encrypt_credentials(
        crypto_key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "legacy-access".to_string(),
            refresh_token: Some("legacy-refresh".to_string()),
            id_token: test_id_token(
                "legacy@example.com",
                Some("org_legacy"),
                Some("user_legacy"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims(
                "legacy@example.com",
                Some("org_legacy"),
                Some("user_legacy"),
            ),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let metadata = load_group_metadata(&state.pool, Some("legacy-group"))
        .await
        .expect("load group metadata");
    assert!(metadata.node_shunt_enabled);
    assert_eq!(metadata.bound_proxy_keys, bound_proxy_keys);

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load oauth account row")
        .expect("oauth account should exist");
    assert_eq!(account.group_name.as_deref(), Some("legacy-group"));

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(completed_session.account_id, Some(account_id));
}
