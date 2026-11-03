use super::super::*;

#[tokio::test]
async fn update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;

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
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

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
    .expect("load repaired oauth account tags");
    assert!(account_tag_ids.is_empty());

    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load repaired group note");
    assert_eq!(group_note.as_deref(), Some("after group note"));

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("reload completed session")
        .expect("completed session should still exist");
    assert_ne!(completed_session.updated_at, created.updated_at);
    assert!(completed_session.consumed_at.is_some());

    let mut second_repair_headers = HeaderMap::new();
    second_repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&repaired.updated_at).expect("valid updated_at header"),
    );
    let second_repair = update_oauth_login_session(
        State(state.clone()),
        second_repair_headers,
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
    .0;

    let repaired_again = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load twice repaired account row")
        .expect("oauth account should still exist");
    assert_eq!(repaired_again.display_name, "Race Final");
    assert_eq!(repaired_again.group_name.as_deref(), Some("race-group"));
    assert_eq!(repaired_again.note.as_deref(), Some("after note"));
    assert_eq!(repaired_again.is_mother, 1);

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
    .expect("load twice repaired oauth account tags");
    assert!(account_tag_ids.is_empty());

    let second_group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load twice repaired group note");
    assert_eq!(second_group_note.as_deref(), Some("after group note"));

    let repaired_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("reload repaired session after second patch")
        .expect("repaired session should still exist");
    assert_eq!(repaired_session.display_name.as_deref(), Some("Race Final"));
    assert_eq!(repaired_session.group_name.as_deref(), Some("race-group"));
    assert_eq!(repaired_session.note.as_deref(), Some("after note"));
    assert_eq!(
        parse_tag_ids_json(repaired_session.tag_ids_json.as_deref()),
        Vec::<i64>::new()
    );
    assert_ne!(second_repair.updated_at, repaired.updated_at);
}

#[tokio::test]
async fn completed_race_repair_preserves_existing_system_tags() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");
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
async fn create_oauth_login_session_rejects_manual_tag_ids() {
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
async fn update_oauth_login_session_rejects_manual_tag_ids() {
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
async fn cleanup_non_system_tags_removes_custom_tags_links_and_session_references() {
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
    .bind(format!(
        "[{custom_tag_id},{{\"kind\":\"object\"}},[\"nested\",{{\"ok\":true}}],true,false,null,\"keep\",{system_tag_id}]"
    ))
    .execute(&state.pool)
    .await
    .expect("seed non-scalar JSON tag values");

    cleanup_non_system_tags(&state.pool)
        .await
        .expect("cleanup non-system tags");

    let remaining_tag_ids = sqlx::query_scalar::<_, i64>(
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
    .expect("load remaining account tags");
    assert_eq!(remaining_tag_ids, vec![system_tag_id]);

    let custom_tag_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_tags
        WHERE id = ?1
        "#,
    )
    .bind(custom_tag_id)
    .fetch_one(&state.pool)
    .await
    .expect("count custom tags");
    assert_eq!(custom_tag_count, 0);

    let system_tag_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_tags
        WHERE id = ?1
        "#,
    )
    .bind(system_tag_id)
    .fetch_one(&state.pool)
    .await
    .expect("count system tags");
    assert_eq!(system_tag_count, 1);

    let stored = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load cleaned login session")
        .expect("cleaned login session should exist");
    assert_eq!(
        stored.tag_ids_json,
        Some(format!(
            r#"[{{"kind":"object"}},["nested",{{"ok":true}}],true,false,null,"keep",{system_tag_id}]"#
        ))
    );

    let scalar_custom_tag_id =
        insert_legacy_custom_tag(&state.pool, "cleanup-scalar", &test_tag_routing_rule()).await;
    sqlx::query("UPDATE pool_oauth_login_sessions SET tag_ids_json = ?2 WHERE login_id = ?1")
        .bind(&created.login_id)
        .bind(scalar_custom_tag_id.to_string())
        .execute(&state.pool)
        .await
        .expect("seed scalar custom tag root");
    cleanup_non_system_tags(&state.pool)
        .await
        .expect("preserve scalar custom tag root");
    let scalar_root = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load scalar-root login session")
        .expect("scalar-root login session should exist");
    assert_eq!(
        scalar_root.tag_ids_json,
        Some(scalar_custom_tag_id.to_string())
    );

    let object_custom_tag_id =
        insert_legacy_custom_tag(&state.pool, "cleanup-object", &test_tag_routing_rule()).await;
    sqlx::query("UPDATE pool_oauth_login_sessions SET tag_ids_json = ?2 WHERE login_id = ?1")
        .bind(&created.login_id)
        .bind(format!(r#"{{"retired":{object_custom_tag_id}}}"#))
        .execute(&state.pool)
        .await
        .expect("seed object custom tag root");
    cleanup_non_system_tags(&state.pool)
        .await
        .expect("preserve object custom tag root");
    let object_root = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load object-root login session")
        .expect("object-root login session should exist");
    assert_eq!(
        object_root.tag_ids_json,
        Some(format!(r#"{{"retired":{object_custom_tag_id}}}"#))
    );
}

#[tokio::test]
async fn update_oauth_login_session_rejects_stale_completed_race_repairs() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

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

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_repairs_after_group_note_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    assert_eq!(completed_session.updated_at, created.updated_at);

    let mut conn = state.pool.acquire().await.expect("acquire group note conn");
    save_group_note_record_conn(
        &mut conn,
        "race-group",
        Some("manual latest group note".to_string()),
    )
    .await
    .expect("save manual latest group note");
    drop(conn);

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repair_err = update_oauth_login_session(
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
    .expect_err("reject repair after group note changes");
    assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        repair_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after repair rejection")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race Before");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("before note"));

    let group_note = sqlx::query_scalar::<_, Option<String>>(
        r#"
            SELECT note
            FROM pool_upstream_account_group_notes
            WHERE group_name = ?1
            "#,
    )
    .bind("race-group")
    .fetch_one(&state.pool)
    .await
    .expect("load preserved group note");
    assert_eq!(group_note.as_deref(), Some("manual latest group note"));
}

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_repairs_after_account_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
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
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let completed_session = load_login_session_by_login_id(&state.pool, &created.login_id)
        .await
        .expect("load completed session")
        .expect("completed session should exist");
    let consumed_at = completed_session
        .consumed_at
        .clone()
        .expect("completed session should record consumed_at");
    let newer_account_updated_at = next_login_session_updated_at(Some(consumed_at.as_str()));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET display_name = ?2,
                note = ?3,
                updated_at = ?4
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("Manual Latest")
    .bind("manual latest note")
    .bind(&newer_account_updated_at)
    .execute(&state.pool)
    .await
    .expect("simulate newer account edit");

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repair_err = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
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
    .expect_err("reject completed repair after account changes");
    assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        repair_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after rejecting stale completed repair")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Manual Latest");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("manual latest note"));
    assert_eq!(account.updated_at, newer_account_updated_at);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_repairs_after_account_email_changes() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let created = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Race Before".to_string()),
            email: Some("race@example.com".to_string()),
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
            display_name: pending_session
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: Some("race@example.com".to_string()),
            verified_email: Some("race@example.com".to_string()),
            session: pending_session,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");

    let newer_account_updated_at = next_login_session_updated_at(Some(&created.updated_at));
    sqlx::query(
        r#"
            UPDATE pool_upstream_accounts
            SET email = ?2,
                updated_at = ?3
            WHERE id = ?1
            "#,
    )
    .bind(account_id)
    .bind("manual.latest@example.com")
    .bind(&newer_account_updated_at)
    .execute(&state.pool)
    .await
    .expect("simulate newer account email edit");

    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    let repair_err = update_oauth_login_session(
        State(state.clone()),
        repair_headers,
        AxumPath(created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Race Before".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value("before note".to_string()),
            group_note: OptionalField::Value("before group note".to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("reject completed repair after account email changes");
    assert_eq!(repair_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        repair_err.1,
        "This login session can no longer be edited.".to_string()
    );

    let account = load_upstream_account_row(&state.pool, account_id)
        .await
        .expect("load account after rejecting stale email repair")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race Before");
    assert_eq!(account.email.as_deref(), Some("manual.latest@example.com"));
    assert_eq!(account.updated_at, newer_account_updated_at);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_completed_failed_and_expired_sessions() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let update_payload = || UpdateOauthLoginSessionRequest {
        display_name: OptionalField::Value("Edited Session".to_string()),
        email: OptionalField::Missing,
        group_name: OptionalField::Value("edited-group".to_string()),
        group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
        group_node_shunt_enabled: OptionalField::Missing,
        group_single_account_rotation_enabled: OptionalField::Missing,
        note: OptionalField::Value("edited note".to_string()),
        group_note: OptionalField::Value("edited group note".to_string()),
        concurrency_limit: OptionalField::Missing,
        tag_ids: OptionalField::Value(vec![]),
        is_mother: OptionalField::Value(false),
        mailbox_session_id: OptionalField::Missing,
        mailbox_address: OptionalField::Missing,
    };

    let completed = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Completed Session".to_string()),
            email: None,
            group_name: Some("completed-group".to_string()),
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
    .expect("create completed session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
        .bind(&completed.login_id)
        .bind(LOGIN_SESSION_STATUS_COMPLETED)
        .execute(&state.pool)
        .await
        .expect("mark session completed");
    let completed_err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(completed.login_id.clone()),
        Json(update_payload()),
    )
    .await
    .expect_err("completed session should reject edits");
    assert_eq!(completed_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        completed_err.1,
        "This login session can no longer be edited."
    );

    let failed = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Failed Session".to_string()),
            email: None,
            group_name: Some("failed-group".to_string()),
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
    .expect("create failed session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
        .bind(&failed.login_id)
        .bind(LOGIN_SESSION_STATUS_FAILED)
        .execute(&state.pool)
        .await
        .expect("mark session failed");
    let failed_err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(failed.login_id.clone()),
        Json(update_payload()),
    )
    .await
    .expect_err("failed session should reject edits");
    assert_eq!(failed_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(failed_err.1, "This login session can no longer be edited.");

    let expired = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some("Expired Session".to_string()),
            email: None,
            group_name: Some("expired-group".to_string()),
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
    .expect("create expired session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET expires_at = ?2 WHERE login_id = ?1")
        .bind(&expired.login_id)
        .bind("2020-01-01T00:00:00Z")
        .execute(&state.pool)
        .await
        .expect("expire session");
    let expired_err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(expired.login_id.clone()),
        Json(update_payload()),
    )
    .await
    .expect_err("expired session should reject edits");
    assert_eq!(expired_err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        expired_err.1,
        "The login session has expired. Please create a new authorization link."
    );

    let expired_session = load_login_session_by_login_id(&state.pool, &expired.login_id)
        .await
        .expect("load expired session")
        .expect("expired session should exist");
    assert_eq!(expired_session.status, LOGIN_SESSION_STATUS_EXPIRED);
}

#[tokio::test]
async fn update_oauth_login_session_rejects_relogin_sessions() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_oauth_account(&state.pool, "Relogin Target").await;
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

    let err = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(relogin.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value("Edited Relogin".to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Missing,
            group_bound_proxy_keys: OptionalField::Missing,
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Missing,
            group_note: OptionalField::Missing,
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(false),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect_err("relogin session should reject edits");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "This login session belongs to an existing account and cannot be edited."
    );
}
