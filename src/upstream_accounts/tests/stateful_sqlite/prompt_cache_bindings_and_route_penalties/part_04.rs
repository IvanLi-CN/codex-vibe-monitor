async fn assert_updated_callback_account(pool: &SqlitePool, account_id: i64, login_id: &str) {
    let account = load_upstream_account_row(pool, account_id)
        .await
        .expect("load oauth account row")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "After Patch");
    assert_eq!(account.group_name.as_deref(), Some("new-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);
    let tags = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id ASC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .expect("load oauth account tags");
    assert!(tags.is_empty());
    let group_note = sqlx::query_scalar::<_, Option<String>>(
        "SELECT note FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("new-group")
    .fetch_one(pool)
    .await
    .expect("load group note");
    assert_eq!(group_note.as_deref(), Some("draft group note"));
    let session = load_login_session_by_login_id(pool, login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(session.account_id, Some(account_id));
}

async fn seed_system_tag_race(
    state: &Arc<AppState>,
) -> (LoginSessionStatusResponse, i64, Vec<i64>) {
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Race Before".to_string()),
            email: None,
            group_name: Some("race-group".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: Some("before note".to_string()),
            group_note: Some("before group note".to_string()),
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
    let pending = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load pending session")
        .expect("pending session should exist");
    let key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let credentials = encrypt_credentials(
        key,
        &StoredCredentials::Oauth(StoredOauthCredentials {
            access_token: "race-access".to_string(),
            refresh_token: Some("race-refresh".to_string()),
            id_token: test_id_token(
                "race@example.com",
                Some("org_race"),
                Some("user_race"),
                Some("team"),
            ),
            token_type: Some("Bearer".to_string()),
        }),
    )
    .expect("encrypt oauth credentials");
    let account_id = persist_oauth_callback_inner(
        state.as_ref(),
        PersistOauthCallbackInput {
            display_name: pending
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials: credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");
    let tags = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id ASC",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load original account tags");
    assert!(!tags.is_empty());
    (created, account_id, tags)
}

#[tokio::test]
pub(crate) async fn updated_oauth_login_session_metadata_is_used_when_callback_persists_account() {
    updated_oauth_login_session_metadata_is_used_when_callback_persists_account_impl().await;
}

async fn updated_oauth_login_session_metadata_is_used_when_callback_persists_account_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = seed_updated_callback_session(&state).await;

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

    assert_updated_callback_account(&state.pool, account_id, &created.login_id).await;
}

async fn seed_updated_callback_session(state: &Arc<AppState>) -> LoginSessionStatusResponse {
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
    created
}

#[tokio::test]
pub(crate) async fn persist_oauth_callback_preserves_group_node_shunt_for_legacy_pending_session() {
    persist_oauth_callback_preserves_group_node_shunt_for_legacy_pending_session_impl().await;
}

async fn persist_oauth_callback_preserves_group_node_shunt_for_legacy_pending_session_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, bound_proxy_keys) = seed_legacy_pending_session(&state).await;
    let account_id = persist_legacy_callback(&state, &created, &bound_proxy_keys).await;

    assert_legacy_callback_result(&state.pool, account_id, &created.login_id, bound_proxy_keys)
        .await;
}

async fn seed_legacy_pending_session(
    state: &Arc<AppState>,
) -> (LoginSessionStatusResponse, Vec<String>) {
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

    (created, bound_proxy_keys)
}

async fn persist_legacy_callback(
    state: &Arc<AppState>,
    created: &LoginSessionStatusResponse,
    _bound_proxy_keys: &[String],
) -> i64 {
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
    persist_oauth_callback_inner(
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
    .expect("persist oauth callback")
}

async fn assert_legacy_callback_result(
    pool: &SqlitePool,
    account_id: i64,
    login_id: &str,
    bound_proxy_keys: Vec<String>,
) {
    let metadata = load_group_metadata(pool, Some("legacy-group"))
        .await
        .expect("load group metadata");
    assert!(metadata.node_shunt_enabled);
    assert_eq!(metadata.bound_proxy_keys, bound_proxy_keys);
    let account = load_upstream_account_row(pool, account_id)
        .await
        .expect("load oauth account row")
        .expect("oauth account should exist");
    assert_eq!(account.group_name.as_deref(), Some("legacy-group"));
    let session = load_login_session_by_login_id(pool, login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(session.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(session.account_id, Some(account_id));
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata()
 {
    update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata_impl().await;
}

async fn update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, account_id) = super::seed_completed_callback_race_session(&state).await;
    let repaired = repair_callback_race_once(&state, &created, account_id).await;

    assert_callback_race_first_result(&state, &created, account_id, &repaired).await;

    let second_repair = repair_callback_race_again(&state, &created, &repaired).await;

    assert_callback_race_second_result(&state, &created, account_id, &second_repair, &repaired)
        .await;
}

async fn repair_callback_race_once(
    state: &Arc<AppState>,
    created: &LoginSessionStatusResponse,
    account_id: i64,
) -> LoginSessionStatusResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repaired = update_oauth_login_session(
        State(state.clone()),
        headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race After".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("after note".to_string()),
            group_note: OptionalField::Value("after group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("repair completed callback race")
    .0;
    assert_eq!(repaired.login_id, created.login_id);
    assert_eq!(repaired.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(repaired.account_id, Some(account_id));
    assert!(repaired.auth_url.is_none());
    assert!(repaired.redirect_uri.is_none());
    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load repaired account row")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race After");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);
    repaired
}

async fn assert_callback_race_first_result(
    state: &Arc<AppState>,
    created: &LoginSessionStatusResponse,
    account_id: i64,
    repaired: &LoginSessionStatusResponse,
) {
    let tags = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id ASC",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load repaired oauth account tags");
    assert!(tags.is_empty());
    let group_note = sqlx::query_scalar::<_, Option<String>>(
        "SELECT note FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load repaired group note");
    assert_eq!(group_note.as_deref(), Some("after group note"));
    let session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("reload completed session")
        .expect("completed session should still exist");
    assert_ne!(session.updated_at, created.updated_at);
    assert!(session.consumed_at.is_some());
    assert_ne!(repaired.updated_at, created.updated_at);
}

async fn repair_callback_race_again(
    state: &Arc<AppState>,
    created: &LoginSessionStatusResponse,
    repaired: &LoginSessionStatusResponse,
) -> LoginSessionStatusResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&repaired.updated_at).expect("valid updated_at header"),
    );
    update_oauth_login_session(
        State(state.clone()),
        headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Final".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
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
    .expect("repair completed callback race again with omitted fields")
    .0
}

async fn assert_callback_race_second_result(
    state: &Arc<AppState>,
    created: &LoginSessionStatusResponse,
    account_id: i64,
    second_repair: &LoginSessionStatusResponse,
    repaired: &LoginSessionStatusResponse,
) {
    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load twice repaired account row")
        .expect("oauth account should still exist");
    assert_eq!(account.display_name, "Race Final");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);
    let tags = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id ASC",
    )
    .bind(account_id)
    .fetch_all(&state.pool)
    .await
    .expect("load twice repaired oauth account tags");
    assert!(tags.is_empty());
    let group_note = sqlx::query_scalar::<_, Option<String>>(
        "SELECT note FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load twice repaired group note");
    assert_eq!(group_note.as_deref(), Some("after group note"));
    let session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("reload repaired session after second patch")
        .expect("repaired session should still exist");
    assert_eq!(session.display_name.as_deref(), Some("Race Final"));
    assert_eq!(session.group_name.as_deref(), Some("race-group"));
    assert_eq!(session.note.as_deref(), Some("after note"));
    assert_eq!(
        parse_tag_ids_json(session.tag_ids_json.as_deref()),
        Vec::<i64>::new()
    );
    assert_ne!(second_repair.updated_at, repaired.updated_at);
}

#[tokio::test]
pub(crate) async fn completed_race_repair_preserves_existing_system_tags() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, account_id, original_tag_ids) = seed_system_tag_race(&state).await;

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repaired = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Latest".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("latest note".to_string()),
            group_note: OptionalField::Value("latest group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("repair completed callback race with system tags")
    .0;

    assert_eq!(repaired.account_id, Some(account_id));

    let repaired_account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load repaired account")
        .expect("oauth account should still exist");
    assert_eq!(repaired_account.display_name, "Race Latest");
    assert_eq!(repaired_account.note.as_deref(), Some("latest note"));
    assert_eq!(repaired_account.is_mother, 1);

    let repaired_tag_ids = sqlx::query_scalar::<_, i64>(
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
    .expect("load repaired account tags");
    assert_eq!(repaired_tag_ids, original_tag_ids);
}

#[tokio::test]
pub(crate) async fn create_oauth_login_session_rejects_manual_tag_ids() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

    let err = create_oauth_login_session(
        State(state),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Tagged Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
            group_note: None,
            concurrency_limit: None,
            account_id: None,
            tag_ids: vec![123],
            is_mother: Some(false),
            mailbox_session_id: None,
            mailbox_address: None,
        }),
    )
    .await
    .expect_err("manual tag ids should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "manual tag assignment is no longer supported; omit tagIds"
    );
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_manual_tag_ids() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
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

    let err = update_oauth_login_session(
        State(state),
        HeaderMap::new(),
        AxumPath(created.login_id),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Missing,
            email: OptionalField::Missing,
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![456]),
            is_mother: OptionalField::Missing,
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("manual tag ids should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "manual tag assignment is no longer supported; omit tagIds"
    );
}

#[tokio::test]
pub(crate) async fn cleanup_non_system_tags_removes_custom_tags_links_and_session_references() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Cleanup Target").await;
    let custom_tag_id =
        insert_legacy_custom_tag(&state.pool, "cleanup-custom", &test_tag_routing_rule()).await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");
    let system_tag_id = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM pool_tags
        WHERE system_key = ?1
        "#,
    )
    .bind("unsupported_model:gpt-5.5")
    .fetch_one(&state.pool)
    .await
    .expect("load system tag id");
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(account_id)
    .bind(custom_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach custom tag");

    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Cleanup Pending".to_string()),
            email: None,
            group_name: Some("alpha".to_string()),
            group_bound_proxy_keys: Some(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: None,
            group_single_account_rotation_enabled: None,
            note: None,
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
        SET tag_ids_json = ?2
        WHERE login_id = ?1
        "#,
    )
    .bind(&created.login_id)
    .bind(serde_json::to_string(&vec![custom_tag_id, system_tag_id]).expect("encode tag ids"))
    .execute(&state.pool)
    .await
    .expect("seed legacy session tag ids");

    cleanup_non_system_tags(&state.pool)
        .await
        .expect("cleanup non-system tags");

    assert_cleanup_tags_result(
        &state.pool,
        account_id,
        custom_tag_id,
        system_tag_id,
        &created.login_id,
    )
    .await;
}

async fn assert_cleanup_tags_result(
    pool: &SqlitePool,
    account_id: i64,
    custom_tag_id: i64,
    system_tag_id: i64,
    login_id: &str,
) {
    let remaining = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id ASC",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .expect("load remaining account tags");
    assert_eq!(remaining, vec![system_tag_id]);
    let custom_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
        .bind(custom_tag_id)
        .fetch_one(pool)
        .await
        .expect("count custom tags");
    assert_eq!(custom_count, 0);
    let system_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
        .bind(system_tag_id)
        .fetch_one(pool)
        .await
        .expect("count system tags");
    assert_eq!(system_count, 1);
    let stored = load_login_session_by_login_id(pool, login_id)
        .await
        .expect("load cleaned login session")
        .expect("cleaned login session should exist");
    assert_eq!(stored.tag_ids_json, None);
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_stale_completed_race_repairs() {
    update_oauth_login_session_rejects_stale_completed_race_repairs_impl().await;
}

async fn update_oauth_login_session_rejects_stale_completed_race_repairs_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, account_id) = super::seed_completed_callback_race_session(&state).await;

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.updated_at, created.updated_at);
    assert!(completed_session.consumed_at.is_some());

    let mut first_headers = HeaderMap::new();
    first_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let first_repair = update_oauth_login_session(
        State(state.clone()),
        first_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Latest".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("latest note".to_string()),
            group_note: OptionalField::Value("latest group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("apply latest repair")
    .0;

    assert_ne!(first_repair.updated_at, created.updated_at);
    assert_eq!(first_repair.account_id, Some(account_id));

    let mut stale_headers = HeaderMap::new();
    stale_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let stale_err = update_oauth_login_session(
        State(state.clone()),
        stale_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Stale".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("stale-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("stale note".to_string()),
            group_note: OptionalField::Value("stale group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("reject stale repair");
    assert_eq!(stale_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        stale_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after stale repair rejection")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race Latest");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("latest note"));
    assert_eq!(account.is_mother, 1);
}

use super::*;
