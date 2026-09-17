use super::*;
use axum::http::header;

struct CompletedCallbackRaceFixture {
    state: Arc<AppState>,
    created: LoginSessionStatusResponse,
    account_id: i64,
}

async fn create_completed_callback_race_fixture() -> CompletedCallbackRaceFixture {
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
    CompletedCallbackRaceFixture {
        state,
        created,
        account_id,
    }
}

async fn repair_completed_callback_race(
    fixture: &CompletedCallbackRaceFixture,
    display_name: &str,
    note: &str,
    group_note: &str,
) -> LoginSessionStatusResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&fixture.created.updated_at)
            .expect("valid updated_at header"),
    );
    update_oauth_login_session(
        State(fixture.state.clone()),
        headers,
        AxumPath(fixture.created.login_id.clone()),
        Json(UpdateOauthLoginSessionRequest {
            display_name: OptionalField::Value(display_name.to_string()),
            email: OptionalField::Missing,
            group_name: OptionalField::Value("race-group".to_string()),
            group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
            group_node_shunt_enabled: OptionalField::Missing,
            group_single_account_rotation_enabled: OptionalField::Missing,
            note: OptionalField::Value(note.to_string()),
            group_note: OptionalField::Value(group_note.to_string()),
            concurrency_limit: OptionalField::Missing,
            tag_ids: OptionalField::Value(vec![]),
            is_mother: OptionalField::Value(true),
            mailbox_session_id: OptionalField::Missing,
            mailbox_address: OptionalField::Missing,
        }),
    )
    .await
    .expect("repair completed callback race")
    .0
}

async fn assert_first_callback_race_repair(
    fixture: &CompletedCallbackRaceFixture,
    repaired: &LoginSessionStatusResponse,
) {
    assert_eq!(repaired.login_id, fixture.created.login_id);
    assert_eq!(repaired.status, LOGIN_SESSION_STATUS_COMPLETED);
    assert_eq!(repaired.account_id, Some(fixture.account_id));
    assert!(repaired.auth_url.is_none());
    assert!(repaired.redirect_uri.is_none());
    let account = load_upstream_account_row(&fixture.state.pool, fixture.account_id)
        .await
        .expect("load repaired account row")
        .expect("oauth account should exist");
    assert_eq!(account.display_name, "Race After");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);
    let tag_ids = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id",
    )
    .bind(fixture.account_id)
    .fetch_all(&fixture.state.pool)
    .await
    .expect("load repaired oauth account tags");
    assert!(tag_ids.is_empty());
    let group_note = sqlx::query_scalar::<_, Option<String>>(
        "SELECT note FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("race-group")
    .fetch_one(&fixture.state.pool)
    .await
    .expect("load repaired group note");
    assert_eq!(group_note.as_deref(), Some("after group note"));
    let session = load_login_session_by_login_id(&fixture.state.pool, &fixture.created.login_id)
        .await
        .expect("reload completed session")
        .expect("completed session should still exist");
    assert_ne!(session.updated_at, fixture.created.updated_at);
    assert!(session.consumed_at.is_some());
}

async fn repair_completed_callback_race_again(
    fixture: &CompletedCallbackRaceFixture,
    repaired: &LoginSessionStatusResponse,
) -> LoginSessionStatusResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&repaired.updated_at).expect("valid updated_at header"),
    );
    update_oauth_login_session(
        State(fixture.state.clone()),
        headers,
        AxumPath(fixture.created.login_id.clone()),
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

async fn assert_second_callback_race_repair(
    fixture: &CompletedCallbackRaceFixture,
    repaired: &LoginSessionStatusResponse,
    second_repair: &LoginSessionStatusResponse,
) {
    let account = load_upstream_account_row(&fixture.state.pool, fixture.account_id)
        .await
        .expect("load twice repaired account row")
        .expect("oauth account should still exist");
    assert_eq!(account.display_name, "Race Final");
    assert_eq!(account.group_name.as_deref(), Some("race-group"));
    assert_eq!(account.note.as_deref(), Some("after note"));
    assert_eq!(account.is_mother, 1);
    let tag_ids = sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id",
    )
    .bind(fixture.account_id)
    .fetch_all(&fixture.state.pool)
    .await
    .expect("load twice repaired oauth account tags");
    assert!(tag_ids.is_empty());
    let group_note = sqlx::query_scalar::<_, Option<String>>(
        "SELECT note FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("race-group")
    .fetch_one(&fixture.state.pool)
    .await
    .expect("load twice repaired group note");
    assert_eq!(group_note.as_deref(), Some("after group note"));
    let session = load_login_session_by_login_id(&fixture.state.pool, &fixture.created.login_id)
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

fn completed_repair_payload(
    display_name: &str,
    group_name: &str,
    note: &str,
    group_note: &str,
    is_mother: bool,
) -> UpdateOauthLoginSessionRequest {
    UpdateOauthLoginSessionRequest {
        display_name: OptionalField::Value(display_name.to_string()),
        email: OptionalField::Missing,
        group_name: OptionalField::Value(group_name.to_string()),
        group_bound_proxy_keys: OptionalField::Value(test_required_group_bound_proxy_keys()),
        group_node_shunt_enabled: OptionalField::Missing,
        group_single_account_rotation_enabled: OptionalField::Missing,
        note: OptionalField::Value(note.to_string()),
        group_note: OptionalField::Value(group_note.to_string()),
        concurrency_limit: OptionalField::Missing,
        tag_ids: OptionalField::Value(vec![]),
        is_mother: OptionalField::Value(is_mother),
        mailbox_session_id: OptionalField::Missing,
        mailbox_address: OptionalField::Missing,
    }
}

async fn reject_completed_repair(
    fixture: &CompletedCallbackRaceFixture,
    payload: UpdateOauthLoginSessionRequest,
) -> String {
    let mut headers = HeaderMap::new();
    headers.insert(
        LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
        header::HeaderValue::from_str(&fixture.created.updated_at)
            .expect("valid updated_at header"),
    );
    let error = update_oauth_login_session(
        State(fixture.state.clone()),
        headers,
        AxumPath(fixture.created.login_id.clone()),
        Json(payload),
    )
    .await
    .expect_err("completed repair should be rejected");
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    error.1
}

async fn load_account_tag_ids(pool: &SqlitePool, account_id: i64) -> Vec<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT tag_id FROM pool_upstream_account_tags WHERE account_id = ?1 ORDER BY tag_id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .expect("load account tags")
}

struct CleanupTagFixture {
    state: Arc<AppState>,
    account_id: i64,
    custom_tag_id: i64,
    system_tag_id: i64,
    login_id: String,
}

async fn create_cleanup_tag_fixture() -> CleanupTagFixture {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Cleanup Target").await;
    let custom_tag_id =
        insert_legacy_custom_tag(&state.pool, "cleanup-custom", &test_tag_routing_rule()).await;
    ensure_account_has_gpt55_unsupported_tag(&state.pool, account_id)
        .await
        .expect("seed system tag");
    let system_tag_id =
        sqlx::query_scalar::<_, i64>("SELECT id FROM pool_tags WHERE system_key = ?1")
            .bind("unsupported_model:gpt-5.5")
            .fetch_one(&state.pool)
            .await
            .expect("load system tag id");
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        "INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
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
    sqlx::query("UPDATE pool_oauth_login_sessions SET tag_ids_json = ?2 WHERE login_id = ?1")
        .bind(&created.login_id)
        .bind(serde_json::to_string(&vec![custom_tag_id, system_tag_id]).expect("encode tag ids"))
        .execute(&state.pool)
        .await
        .expect("seed legacy session tag ids");
    CleanupTagFixture {
        state,
        account_id,
        custom_tag_id,
        system_tag_id,
        login_id: created.login_id,
    }
}

async fn assert_cleanup_tag_fixture(fixture: &CleanupTagFixture) {
    assert_eq!(
        load_account_tag_ids(&fixture.state.pool, fixture.account_id).await,
        vec![fixture.system_tag_id]
    );
    for (tag_id, expected_count) in [(fixture.custom_tag_id, 0), (fixture.system_tag_id, 1)] {
        let count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM pool_tags WHERE id = ?1")
            .bind(tag_id)
            .fetch_one(&fixture.state.pool)
            .await
            .expect("count cleaned tag");
        assert_eq!(count, expected_count);
    }
    let stored = load_login_session_by_login_id(&fixture.state.pool, &fixture.login_id)
        .await
        .expect("load cleaned login session")
        .expect("cleaned login session should exist");
    assert_eq!(stored.tag_ids_json, None);
}

async fn create_session_with_status(
    state: &Arc<AppState>,
    display_name: &str,
    group_name: &str,
    status: &str,
) -> String {
    let created = create_oauth_login_session(
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
    .expect("create session rejection seed")
    .0;
    sqlx::query("UPDATE pool_oauth_login_sessions SET status = ?2 WHERE login_id = ?1")
        .bind(&created.login_id)
        .bind(status)
        .execute(&state.pool)
        .await
        .expect("mark session status");
    created.login_id
}

async fn assert_session_update_rejected(
    state: &Arc<AppState>,
    login_id: &str,
    payload: UpdateOauthLoginSessionRequest,
    message: &str,
) {
    let error = update_oauth_login_session(
        State(state.clone()),
        HeaderMap::new(),
        AxumPath(login_id.to_string()),
        Json(payload),
    )
    .await
    .expect_err("session update should be rejected");
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert_eq!(error.1, message);
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_repairs_completed_callback_race_with_latest_metadata()
 {
    let fixture = create_completed_callback_race_fixture().await;
    let repaired =
        repair_completed_callback_race(&fixture, "Race After", "after note", "after group note")
            .await;
    assert_first_callback_race_repair(&fixture, &repaired).await;
    let second_repair = repair_completed_callback_race_again(&fixture, &repaired).await;
    assert_second_callback_race_repair(&fixture, &repaired, &second_repair).await;
}

#[tokio::test]
pub(crate) async fn completed_race_repair_preserves_existing_system_tags() {
    let fixture = create_completed_callback_race_fixture().await;
    ensure_account_has_gpt55_unsupported_tag(&fixture.state.pool, fixture.account_id)
        .await
        .expect("seed system tag");
    let original_tag_ids = load_account_tag_ids(&fixture.state.pool, fixture.account_id).await;
    assert!(!original_tag_ids.is_empty());
    let repaired =
        repair_completed_callback_race(&fixture, "Race Latest", "latest note", "latest group note")
            .await;
    assert_eq!(repaired.account_id, Some(fixture.account_id));
    let repaired_account = load_upstream_account_row(&fixture.state.pool, fixture.account_id)
        .await
        .expect("load repaired account")
        .expect("oauth account should still exist");
    assert_eq!(repaired_account.display_name, "Race Latest");
    assert_eq!(repaired_account.note.as_deref(), Some("latest note"));
    assert_eq!(repaired_account.is_mother, 1);

    let repaired_tag_ids = load_account_tag_ids(&fixture.state.pool, fixture.account_id).await;
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
    let fixture = create_cleanup_tag_fixture().await;
    cleanup_non_system_tags(&fixture.state.pool)
        .await
        .expect("cleanup non-system tags");
    assert_cleanup_tag_fixture(&fixture).await;
}

#[tokio::test]
pub(crate) async fn update_oauth_login_session_rejects_stale_completed_race_repairs() {
    let fixture = create_completed_callback_race_fixture().await;
    let state = fixture.state.clone();
    let created = fixture.created.clone();
    let account_id = fixture.account_id;

    let first_repair =
        repair_completed_callback_race(&fixture, "Race Latest", "latest note", "latest group note")
            .await;

    assert_ne!(first_repair.updated_at, created.updated_at);
    assert_eq!(first_repair.account_id, Some(account_id));

    let stale_err = reject_completed_repair(
        &fixture,
        completed_repair_payload(
            "Race Stale",
            "stale-group",
            "stale note",
            "stale group note",
            false,
        ),
    )
    .await;
    assert_eq!(
        stale_err,
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
pub(crate) async fn update_oauth_login_session_rejects_completed_repairs_after_group_note_changes()
{
    let fixture = create_completed_callback_race_fixture().await;
    let state = fixture.state.clone();
    let account_id = fixture.account_id;

    let mut conn = state.pool.acquire().await.expect("acquire group note conn");
    save_group_note_record_conn(
        &mut conn,
        "race-group",
        Some("manual latest group note".to_string()),
    )
    .await
    .expect("save manual latest group note");
    drop(conn);

    let repair_err = reject_completed_repair(
        &fixture,
        completed_repair_payload(
            "Race Latest",
            "race-group",
            "latest note",
            "latest group note",
            true,
        ),
    )
    .await;
    assert_eq!(
        repair_err,
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
    let fixture = create_completed_callback_race_fixture().await;
    let state = fixture.state.clone();
    let created = fixture.created.clone();
    let account_id = fixture.account_id;
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

    let repair_err = reject_completed_repair(
        &fixture,
        completed_repair_payload(
            "Race Stale",
            "stale-group",
            "stale note",
            "stale group note",
            false,
        ),
    )
    .await;
    assert_eq!(
        repair_err,
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
    let fixture = create_completed_callback_race_fixture().await;
    let state = fixture.state.clone();
    let created = fixture.created.clone();
    let account_id = fixture.account_id;

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

    let repair_err = reject_completed_repair(
        &fixture,
        completed_repair_payload(
            "Race Before",
            "race-group",
            "before note",
            "before group note",
            false,
        ),
    )
    .await;
    assert_eq!(
        repair_err,
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
pub(crate) async fn update_oauth_login_session_rejects_completed_failed_and_expired_sessions() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let update_payload = || {
        completed_repair_payload(
            "Edited Session",
            "edited-group",
            "edited note",
            "edited group note",
            false,
        )
    };
    let completed = create_session_with_status(
        &state,
        "Completed Session",
        "completed-group",
        LOGIN_SESSION_STATUS_COMPLETED,
    )
    .await;
    assert_session_update_rejected(
        &state,
        &completed,
        update_payload(),
        "This login session can no longer be edited.",
    )
    .await;
    let failed = create_session_with_status(
        &state,
        "Failed Session",
        "failed-group",
        LOGIN_SESSION_STATUS_FAILED,
    )
    .await;
    assert_session_update_rejected(
        &state,
        &failed,
        update_payload(),
        "This login session can no longer be edited.",
    )
    .await;

    let expired = create_session_with_status(
        &state,
        "Expired Session",
        "expired-group",
        LOGIN_SESSION_STATUS_PENDING,
    )
    .await;
    sqlx::query("UPDATE pool_oauth_login_sessions SET expires_at = ?2 WHERE login_id = ?1")
        .bind(&expired)
        .bind("2020-01-01T00:00:00Z")
        .execute(&state.pool)
        .await
        .expect("expire session");
    assert_session_update_rejected(
        &state,
        &expired,
        update_payload(),
        "The login session has expired. Please create a new authorization link.",
    )
    .await;

    let expired_session = load_login_session_by_login_id(&state.pool, &expired)
        .await
        .expect("load expired session")
        .expect("expired session should exist");
    assert_eq!(expired_session.status, LOGIN_SESSION_STATUS_EXPIRED);
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
