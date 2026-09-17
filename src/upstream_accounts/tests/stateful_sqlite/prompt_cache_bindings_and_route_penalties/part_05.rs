async fn seed_completed_race_session(state: &Arc<AppState>) -> (LoginSessionStatusResponse, i64) {
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
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let encrypted = encrypt_credentials(
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
            display_name: pending
                .display_name
                .clone()
                .expect("display name should be stored"),
            chosen_email: None,
            verified_email: None,
            session: pending,
            claims: test_claims("race@example.com", Some("org_race"), Some("user_race")),
            encrypted_credentials: encrypted,
            has_refresh_token: true,
            token_expires_at: "2026-04-01T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("persist oauth callback");
    (created, account_id)
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_completed_repairs_after_group_note_changes()
{
    update_oauth_login_session_rejects_completed_repairs_after_group_note_changes_impl().await;
}

async fn update_oauth_login_session_rejects_completed_repairs_after_group_note_changes_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, account_id) = seed_completed_race_session(&state).await;

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
pub(crate) async fn update_oauth_login_session_rejects_completed_repairs_after_account_changes() {
    update_oauth_login_session_rejects_completed_repairs_after_account_changes_impl().await;
}

async fn update_oauth_login_session_rejects_completed_repairs_after_account_changes_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, account_id) = seed_completed_race_session(&state).await;

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
pub(crate) async fn update_oauth_login_session_rejects_completed_repairs_after_account_email_changes()
 {
    update_oauth_login_session_rejects_completed_repairs_after_account_email_changes_impl().await;
}

async fn update_oauth_login_session_rejects_completed_repairs_after_account_email_changes_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let (created, account_id) = seed_completed_email_race_session(&state).await;

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

    let repair_err = reject_completed_email_repair(&state, &created).await;
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

async fn seed_completed_email_race_session(
    state: &Arc<AppState>,
) -> (LoginSessionStatusResponse, i64) {
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

    (created, account_id)
}

async fn reject_completed_email_repair(
    state: &Arc<AppState>,
    created: &LoginSessionStatusResponse,
) -> (StatusCode, String) {
    let mut repair_headers = HeaderMap::new();
    repair_headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&created.updated_at).expect("valid updated_at header"),
    );
    update_oauth_login_session(
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
    .expect_err("reject completed repair after account email changes")
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_completed_failed_and_expired_sessions() {
    update_oauth_login_session_rejects_completed_failed_and_expired_sessions_impl().await;
}

async fn update_oauth_login_session_rejects_completed_failed_and_expired_sessions_impl() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    assert_terminal_session_rejects_update(
        &state,
        "Completed Session",
        "completed-group",
        LOGIN_SESSION_STATUS_COMPLETED,
        "completed session should reject edits",
    )
    .await;
    assert_terminal_session_rejects_update(
        &state,
        "Failed Session",
        "failed-group",
        LOGIN_SESSION_STATUS_FAILED,
        "failed session should reject edits",
    )
    .await;
    assert_expired_session_rejects_update(&state).await;
}

fn edit_session_payload() -> UpdateOauthLoginSessionRequest {
    UpdateOauthLoginSessionRequest {
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
    }
}

async fn assert_terminal_session_rejects_update(
    state: &Arc<AppState>,
    display_name: &str,
    group_name: &str,
    status: &str,
    expectation: &str,
) {
    let session = create_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        Json(CreateOauthLoginSessionRequest {
            display_name: Some(display_name.to_string()),
            email: None,
            group_name: Some(group_name.to_string()),
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
    .expect("create terminal session seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
        .bind(&session.login_id)
        .bind(status)
        .execute(&state.pool)
        .await
        .expect("mark terminal session");
    let error = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(session.login_id),
        Json(edit_session_payload()),
    )
    .await
    .expect_err(expectation);
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(error.1, "This login session can no longer be edited.");
}

async fn assert_expired_session_rejects_update(state: &Arc<AppState>) {
    let session = create_oauth_login_session(
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
        .bind(&session.login_id)
        .bind("2020-01-01T00:00:00Z")
        .execute(&state.pool)
        .await
        .expect("expire session");
    let error = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(session.login_id.clone()),
        Json(edit_session_payload()),
    )
    .await
    .expect_err("expired session should reject edits");
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        error.1,
        "The login session has expired. Please create a new authorization link."
    );
    let expired = load_login_session_by_login_id(&state.pool, &session.login_id)
        .await
        .expect("load expired session")
        .expect("expired session should exist");
    assert_eq!(expired.status, LOGIN_SESSION_STATUS_EXPIRED);
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_relogin_sessions() {
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

use super::*;
