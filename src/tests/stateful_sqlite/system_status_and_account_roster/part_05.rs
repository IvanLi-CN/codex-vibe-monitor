#[tokio::test]
pub(crate) async fn list_upstream_accounts_includes_archived_last_activity_at() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("create archive activity pool");
    let account_id = 17_i64;

    sqlx::query(
        "CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY, last_activity_at TEXT)",
    )
    .execute(&pool)
    .await
    .expect("create accounts table");
    sqlx::query("CREATE TABLE codex_invocations (occurred_at TEXT NOT NULL, payload TEXT)")
        .execute(&pool)
        .await
        .expect("create active invocation table");
    sqlx::query("INSERT INTO pool_upstream_accounts (id, last_activity_at) VALUES (?1, ?2)")
        .bind(account_id)
        .bind("2026-03-12 07:05:00")
        .execute(&pool)
        .await
        .expect("seed persisted last activity");

    let last_activity = load_account_last_activity_map(&pool, &[account_id])
        .await
        .expect("load last activity map");

    assert_eq!(
        last_activity.get(&account_id).map(String::as_str),
        Some("2026-03-12 07:05:00")
    );
}

#[tokio::test]
pub(crate) async fn create_api_key_account_enforces_single_mother_per_group() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let first_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "sk-primary",
        Some("prod"),
        Some(true),
        None,
    )
    .await;

    let second_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "sk-secondary",
        Some("prod"),
        Some(true),
        None,
    )
    .await;

    let first_is_mother: i64 =
        sqlx::query_scalar("SELECT is_mother FROM pool_upstream_accounts WHERE id = ?1")
            .bind(first_id)
            .fetch_one(&state.pool)
            .await
            .expect("load first mother flag");
    let second_is_mother: i64 =
        sqlx::query_scalar("SELECT is_mother FROM pool_upstream_accounts WHERE id = ?1")
            .bind(second_id)
            .fetch_one(&state.pool)
            .await
            .expect("load second mother flag");

    assert_eq!(first_is_mother, 0);
    assert_eq!(second_is_mother, 1);
}

#[tokio::test]
pub(crate) async fn create_api_key_account_persists_upstream_base_url() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Gateway Key",
        "apiKey": "sk-gateway",
        "upstreamBaseUrl": "https://proxy.example.com/gateway",
    }))
    .expect("deserialize api key account request");
    let Json(detail) =
        create_api_key_account(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("create api key account");

    let detail_json = serde_json::to_value(detail).expect("serialize detail");
    assert_eq!(
        detail_json["upstreamBaseUrl"].as_str(),
        Some("https://proxy.example.com/gateway")
    );

    let stored: Option<String> = sqlx::query_scalar(
        "SELECT upstream_base_url FROM pool_upstream_accounts WHERE display_name = ?1",
    )
    .bind("Gateway Key")
    .fetch_one(&state.pool)
    .await
    .expect("load stored upstream base url");
    assert_eq!(stored.as_deref(), Some("https://proxy.example.com/gateway"));

    let bound_proxy_keys: Option<String> = sqlx::query_scalar(
        "SELECT bound_proxy_keys_json FROM pool_upstream_accounts WHERE display_name = ?1",
    )
    .bind("Gateway Key")
    .fetch_one(&state.pool)
    .await
    .expect("load stored default transit proxy binding");
    assert_eq!(
        decode_group_bound_proxy_keys_json(bound_proxy_keys.as_deref()),
        vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
    );
}

#[tokio::test]
pub(crate) async fn create_api_key_account_rejects_empty_transit_proxy_bindings() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "No Transit Proxy",
        "apiKey": "sk-no-transit-proxy",
        "boundProxyKeys": [],
    }))
    .expect("deserialize empty transit proxy request");

    let err = create_api_key_account(State(state), HeaderMap::new(), Json(payload))
        .await
        .expect_err("empty transit proxy bindings must be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "API Key accounts require at least one bound proxy node"
    );
}

#[tokio::test]
pub(crate) async fn create_api_key_account_rejects_null_transit_proxy_bindings() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Null Transit Proxy",
        "apiKey": "sk-null-transit-proxy",
        "boundProxyKeys": null,
    }))
    .expect("deserialize null transit proxy request");

    let err = create_api_key_account(State(state), HeaderMap::new(), Json(payload))
        .await
        .expect_err("null transit proxy bindings must be rejected");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "API Key accounts require at least one bound proxy node"
    );
}

#[tokio::test]
pub(crate) async fn update_api_key_account_rejects_empty_transit_proxy_bindings() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Existing Transit", "sk-existing-transit").await;
    let payload: UpdateUpstreamAccountRequest = serde_json::from_value(json!({
        "boundProxyKeys": [],
    }))
    .expect("deserialize empty transit proxy update");

    let err = update_upstream_account(
        State(state),
        HeaderMap::new(),
        axum::extract::Path(account_id),
        Json(payload),
    )
    .await
    .expect_err("empty transit proxy bindings must be rejected on update");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "API Key accounts require at least one bound proxy node"
    );
}

#[tokio::test]
pub(crate) async fn update_api_key_account_rejects_null_transit_proxy_bindings() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Null Transit Update", "sk-null-update").await;
    let payload: UpdateUpstreamAccountRequest = serde_json::from_value(json!({
        "boundProxyKeys": null,
    }))
    .expect("deserialize null transit proxy update");

    let err = update_upstream_account(
        State(state),
        HeaderMap::new(),
        axum::extract::Path(account_id),
        Json(payload),
    )
    .await
    .expect_err("null transit proxy bindings must be rejected on update");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "API Key accounts require at least one bound proxy node"
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_can_clear_upstream_base_url_with_null_payload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Gateway Key",
        "sk-gateway",
        None,
        None,
        Some("https://proxy.example.com/gateway"),
    )
    .await;

    let payload: UpdateUpstreamAccountRequest = serde_json::from_value(json!({
        "upstreamBaseUrl": null,
    }))
    .expect("deserialize update request");
    let Json(detail) = update_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path(account_id),
        Json(payload),
    )
    .await
    .expect("clear upstream base url");

    let detail_json = serde_json::to_value(detail).expect("serialize detail");
    assert!(detail_json["upstreamBaseUrl"].is_null());

    let stored: Option<String> =
        sqlx::query_scalar("SELECT upstream_base_url FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load cleared upstream base url");
    assert_eq!(stored, None);
}

struct DeleteAccountFixture {
    account_id: i64,
    sticky_key: String,
    stale_generation: i64,
}

async fn seed_delete_account_tag(pool: &SqlitePool, account_id: i64, now_iso: &str) -> i64 {
    let tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name, allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 1, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("delete-tag")
    .bind(now_iso)
    .fetch_one(pool)
    .await
    .expect("insert tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(account_id)
    .bind(tag_id)
    .bind(now_iso)
    .execute(pool)
    .await
    .expect("insert account tag link");
    tag_id
}

async fn seed_delete_account_session(pool: &SqlitePool, account_id: i64, now_iso: &str) {
    sqlx::query(
        r#"
        INSERT INTO pool_oauth_login_sessions (
            login_id, account_id, display_name, group_name, is_mother, note, tag_ids_json, group_note,
            state, pkce_verifier, redirect_uri, status, auth_url, error_message, expires_at, consumed_at,
            created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, 0, NULL, NULL, NULL,
            ?5, ?6, ?7, ?8, ?9, NULL, ?10, NULL,
            ?11, ?11
        )
        "#,
    )
    .bind("login-delete-target")
    .bind(account_id)
    .bind("Delete Target")
    .bind("prod")
    .bind("state-delete-target")
    .bind("pkce-delete-target")
    .bind("https://example.com/callback")
    .bind("completed")
    .bind("https://example.com/auth")
    .bind(now_iso)
    .bind(now_iso)
    .execute(pool)
    .await
    .expect("insert oauth login session");
}

async fn seed_delete_account_prompt_cache(pool: &SqlitePool, account_id: i64) {
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_conversation_bindings (
            prompt_cache_key, binding_kind, group_name, upstream_account_id
        ) VALUES (?1, 'upstream_account', NULL, ?2)
        "#,
    )
    .bind("delete-target-prompt-cache-binding")
    .bind(account_id)
    .execute(pool)
    .await
    .expect("insert prompt cache account binding");
    sqlx::query(
        r#"
        INSERT INTO prompt_cache_encrypted_session_owners (
            prompt_cache_key, owner_upstream_account_id
        ) VALUES (?1, ?2)
        "#,
    )
    .bind("delete-target-prompt-cache-owner")
    .bind(account_id)
    .execute(pool)
    .await
    .expect("insert prompt cache encrypted owner");
}

async fn seed_delete_account_group_metadata(pool: &SqlitePool, account_id: i64, now_iso: &str) {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name, note, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        ON CONFLICT(group_name) DO UPDATE SET
            note = excluded.note,
            updated_at = excluded.updated_at
        "#,
    )
    .bind("prod")
    .bind("cleanup me")
    .bind(now_iso)
    .execute(pool)
    .await
    .expect("insert group note");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_limit_samples (
            account_id, captured_at, limit_id, limit_name, plan_type,
            primary_used_percent, primary_window_minutes, primary_resets_at,
            secondary_used_percent, secondary_window_minutes, secondary_resets_at,
            credits_has_credits, credits_unlimited, credits_balance
        ) VALUES (
            ?1, ?2, 'primary', 'Primary', 'team',
            12.5, 300, ?2, 25.0, 10080, ?2,
            1, 0, '42'
        )
        "#,
    )
    .bind(account_id)
    .bind(now_iso)
    .execute(pool)
    .await
    .expect("insert limit sample");
}

async fn seed_delete_account_fixture(state: &Arc<AppState>) -> DeleteAccountFixture {
    let account_id = insert_test_pool_api_key_account_with_options(
        state,
        "Delete Target",
        "sk-delete-target",
        Some("prod"),
        Some(false),
        None,
    )
    .await;
    let now_iso = format_utc_iso(Utc::now());
    let sticky_key = "delete-target-sticky-key".to_string();
    upsert_sticky_route(&state.pool, &sticky_key, account_id, &now_iso)
        .await
        .expect("insert sticky route");
    let stale_generation = load_sticky_affinity_generation(&state.pool, &sticky_key)
        .await
        .expect("load sticky generation before account deletion");
    seed_delete_account_tag(&state.pool, account_id, &now_iso).await;
    seed_delete_account_session(&state.pool, account_id, &now_iso).await;
    seed_delete_account_prompt_cache(&state.pool, account_id).await;
    seed_delete_account_group_metadata(&state.pool, account_id, &now_iso).await;
    DeleteAccountFixture {
        account_id,
        sticky_key,
        stale_generation,
    }
}

async fn assert_deleted_account_sticky(state: &Arc<AppState>, fixture: &DeleteAccountFixture) {
    assert!(
        load_sticky_route(&state.pool, &fixture.sticky_key)
            .await
            .expect("load deleted account sticky route")
            .is_none()
    );
    let deleted_generation = load_sticky_affinity_generation(&state.pool, &fixture.sticky_key)
        .await
        .expect("load sticky generation after account deletion");
    assert_eq!(deleted_generation, fixture.stale_generation + 1);
    record_pool_route_success_with_affinity_generation(
        &state.pool,
        fixture.account_id,
        Utc::now(),
        Some(&fixture.sticky_key),
        Some(&fixture.sticky_key),
        Some("late-after-account-delete"),
        Some(fixture.stale_generation),
    )
    .await
    .expect("late completion should remain a successful route outcome");
    assert!(
        load_sticky_route(&state.pool, &fixture.sticky_key)
            .await
            .expect("load sticky route after stale completion")
            .is_none()
    );
}

async fn assert_deleted_account_core(state: &Arc<AppState>, fixture: &DeleteAccountFixture) {
    let remaining_accounts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_accounts WHERE id = ?1")
            .bind(fixture.account_id)
            .fetch_one(&state.pool)
            .await
            .expect("count remaining accounts");
    assert_eq!(remaining_accounts, 1);
    let deleted_at: Option<String> =
        sqlx::query_scalar("SELECT deleted_at FROM pool_upstream_accounts WHERE id = ?1")
            .bind(fixture.account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load soft-delete timestamp");
    assert!(deleted_at.is_some());
    let stored_credentials: Option<String> = sqlx::query_scalar(
        "SELECT encrypted_credentials FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(fixture.account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load cleared credentials");
    assert_eq!(stored_credentials, None);
    let (live_backfill_completed, archive_backfill_completed): (i64, i64) = sqlx::query_as(
        "SELECT last_activity_live_backfill_completed, last_activity_archive_backfill_completed FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(fixture.account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load completed backfill flags");
    assert_eq!(live_backfill_completed, 1);
    assert_eq!(archive_backfill_completed, 1);
}

async fn assert_deleted_account_relations(state: &Arc<AppState>, fixture: &DeleteAccountFixture) {
    let remaining_tag_links: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_account_tags WHERE account_id = ?1")
            .bind(fixture.account_id)
            .fetch_one(&state.pool)
            .await
            .expect("count remaining tag links");
    assert_eq!(remaining_tag_links, 0);
    let remaining_sessions: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pool_oauth_login_sessions WHERE account_id = ?1")
            .bind(fixture.account_id)
            .fetch_one(&state.pool)
            .await
            .expect("count remaining login sessions");
    assert_eq!(remaining_sessions, 0);
    let remaining_prompt_cache_bindings: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_conversation_bindings WHERE upstream_account_id = ?1",
    )
    .bind(fixture.account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count remaining prompt cache bindings");
    assert_eq!(remaining_prompt_cache_bindings, 0);
}

async fn assert_deleted_account_samples_and_group_note(
    state: &Arc<AppState>,
    fixture: &DeleteAccountFixture,
) {
    let remaining_prompt_cache_owners: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM prompt_cache_encrypted_session_owners WHERE owner_upstream_account_id = ?1",
    )
    .bind(fixture.account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count remaining prompt cache owners");
    assert_eq!(remaining_prompt_cache_owners, 0);
    let remaining_samples: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_limit_samples WHERE account_id = ?1",
    )
    .bind(fixture.account_id)
    .fetch_one(&state.pool)
    .await
    .expect("count remaining limit samples");
    assert_eq!(remaining_samples, 0);
    let remaining_group_notes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("prod")
    .fetch_one(&state.pool)
    .await
    .expect("count remaining group notes");
    assert_eq!(remaining_group_notes, 1);
}

#[tokio::test]
pub(crate) async fn delete_upstream_account_keeps_persisted_group_catalog_rows_after_last_member_is_removed()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let fixture = seed_delete_account_fixture(&state).await;

    let status = delete_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path(fixture.account_id),
    )
    .await
    .expect("delete upstream account");
    assert_eq!(status, StatusCode::NO_CONTENT);

    assert_deleted_account_sticky(&state, &fixture).await;
    assert_deleted_account_core(&state, &fixture).await;
    assert_deleted_account_relations(&state, &fixture).await;
    assert_deleted_account_samples_and_group_note(&state, &fixture).await;
}

#[tokio::test]
pub(crate) async fn create_api_key_account_rejects_invalid_upstream_base_url() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateApiKeyAccountRequest = serde_json::from_value(json!({
        "displayName": "Broken Key",
        "apiKey": "sk-broken",
        "upstreamBaseUrl": "not-a-url",
    }))
    .expect("deserialize api key account request");

    let err = create_api_key_account(State(state), HeaderMap::new(), Json(payload))
        .await
        .expect_err("invalid upstream base url should fail");
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(err.1, "upstreamBaseUrl must be a valid absolute URL");
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_rejects_bindings_without_selectable_nodes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "boundProxyKeys": ["fpn_missing_legacy_vless"]
    }))
    .expect("deserialize update upstream account group request");
    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(payload),
    )
    .await
    .expect_err("group binding without selectable nodes should fail");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        err.1,
        "select at least one available proxy node or clear bindings before saving"
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_canonicalizes_historical_runtime_keys_to_logical_binding_keys()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let current_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fcurrent&sni=current.example.com#东京专线".to_string();
    let legacy_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Flegacy&sni=legacy.example.com#东京专线".to_string();
    let settings = ForwardProxySettings {
        proxy_urls: vec![current_proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist current forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        for endpoint in &mut manager.endpoints {
            endpoint.endpoint_url = Some(
                Url::parse("socks5://127.0.0.1:11083")
                    .expect("parse synthesized binding endpoint url"),
            );
        }
    }

    let legacy_proxy_key =
        normalize_single_proxy_key(&legacy_proxy_url).expect("normalize legacy runtime proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(
                normalize_share_link_scheme(&legacy_proxy_url, "vless")
                    .expect("normalize legacy share link"),
            ),
            weight: 0.61,
            success_ema: 0.81,
            latency_ema_ms: Some(140.0),
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist legacy runtime state for metadata history");

    let payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "boundProxyKeys": [legacy_proxy_key]
    }))
    .expect("deserialize update upstream account group request");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(payload),
    )
    .await
    .expect("legacy runtime key should canonicalize during save");

    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&current_proxy_url, None)
            .expect("binding parts from current proxy url"),
    )[0]
    .clone();
    let updated_value = serde_json::to_value(&updated).expect("serialize updated group summary");
    assert_eq!(
        updated_value
            .get("boundProxyKeys")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        vec![Value::String(binding_key.clone())]
    );
    let stored_json: String = sqlx::query_scalar(
        "SELECT bound_proxy_keys_json FROM pool_upstream_account_group_notes WHERE group_name = ?1",
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load stored group metadata after update");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&stored_json)
            .expect("decode stored bound proxy keys json"),
        vec![binding_key]
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_upserts_missing_group_and_still_validates_bindings()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    let payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "boundProxyKeys": ["fpn_missing_legacy_vless"]
    }))
    .expect("deserialize update upstream account group request");
    let err = update_upstream_account_group(
        State(state),
        HeaderMap::new(),
        axum::extract::Path("missing-group".to_string()),
        Json(payload),
    )
    .await
    .expect_err("missing group should still validate bindings");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
    assert!(!err.1.trim().is_empty());
}

#[tokio::test]
pub(crate) async fn ensure_schema_adds_group_upstream_429_retry_columns_with_disabled_defaults() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE pool_upstream_account_group_notes (
            group_name TEXT PRIMARY KEY,
            note TEXT NOT NULL,
            bound_proxy_keys_json TEXT NOT NULL DEFAULT '[]',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy group metadata table");

    ensure_schema(&pool)
        .await
        .expect("schema migration should succeed");

    let columns = load_sqlite_table_columns(&pool, "pool_upstream_account_group_notes")
        .await
        .expect("load migrated columns");
    assert!(columns.contains("upstream_429_retry_enabled"));
    assert!(columns.contains("upstream_429_max_retries"));

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_group_notes (
            group_name,
            note,
            bound_proxy_keys_json,
            created_at,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?4)
        "#,
    )
    .bind("latam")
    .bind("Legacy group")
    .bind("[]")
    .bind(&now_iso)
    .execute(&pool)
    .await
    .expect("insert migrated group metadata");

    let retry_settings = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT upstream_429_retry_enabled, upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&pool)
    .await
    .expect("load retry settings");
    assert_eq!(retry_settings.0, 0);
    assert_eq!(retry_settings.1, 0);
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_preserves_upstream_429_retry_settings_when_omitted()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let initial_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "note": "LATAM premium",
        "upstream429RetryEnabled": true,
        "upstream429MaxRetries": 4
    }))
    .expect("deserialize initial group payload");
    let Json(initial_saved) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(initial_payload),
    )
    .await
    .expect("save group retry settings");
    let initial_saved_json = serde_json::to_value(initial_saved).expect("serialize saved group");
    assert_eq!(
        initial_saved_json["upstream429RetryEnabled"].as_bool(),
        Some(true)
    );
    assert_eq!(
        initial_saved_json["upstream429MaxRetries"].as_u64(),
        Some(4)
    );

    let legacy_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "note": "LATAM refreshed"
    }))
    .expect("deserialize legacy payload");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(legacy_payload),
    )
    .await
    .expect("legacy update should preserve retry settings");
    let updated_json = serde_json::to_value(updated).expect("serialize updated group");
    assert_eq!(updated_json["note"].as_str(), Some("LATAM refreshed"));
    assert_eq!(
        updated_json["upstream429RetryEnabled"].as_bool(),
        Some(true)
    );
    assert_eq!(updated_json["upstream429MaxRetries"].as_u64(), Some(4));

    let persisted = sqlx::query_as::<_, (String, i64, i64)>(
        r#"
        SELECT note, upstream_429_retry_enabled, upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load persisted group retry settings");
    assert_eq!(persisted.0, "LATAM refreshed");
    assert_eq!(persisted.1, 1);
    assert_eq!(persisted.2, 4);
}

#[tokio::test]
pub(crate) async fn update_upstream_account_group_enabling_retry_defaults_missing_retry_count_to_one()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let enable_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "upstream429RetryEnabled": true
    }))
    .expect("deserialize enable payload");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(enable_payload),
    )
    .await
    .expect("enable group retry settings");
    let updated_json = serde_json::to_value(updated).expect("serialize updated group");
    assert_eq!(
        updated_json["upstream429RetryEnabled"].as_bool(),
        Some(true)
    );
    assert_eq!(updated_json["upstream429MaxRetries"].as_u64(), Some(1));

    let persisted = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT upstream_429_retry_enabled, upstream_429_max_retries
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load persisted group retry settings");
    assert_eq!(persisted.0, 1);
    assert_eq!(persisted.1, 1);
}

use super::*;
