pub(crate) fn encrypt_test_oauth_credentials(access_token: &str) -> String {
    let key = Sha256::digest(b"test-upstream-account-secret");
    let cipher = Aes256Gcm::new(&key);
    let plaintext = serde_json::to_vec(&json!({
        "kind": "oauth",
        "accessToken": access_token,
        "refreshToken": "refresh-token",
        "idToken": "header.payload.signature",
        "tokenType": "Bearer",
    }))
    .expect("serialize oauth credentials");
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let nonce_value = aes_gcm::Nonce::from(nonce);
    let ciphertext = cipher
        .encrypt(&nonce_value, plaintext.as_ref())
        .expect("encrypt oauth credentials");
    json!({
        "v": 1,
        "nonce": BASE64_STANDARD.encode(nonce),
        "ciphertext": BASE64_STANDARD.encode(ciphertext),
    })
    .to_string()
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_includes_last_activity_at() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("account-last-activity")
    .bind("2026-03-11 20:35:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(42_i64)
    .bind(0.12_f64)
    .bind(
        json!({
            "upstreamAccountId": account_id,
        })
        .to_string(),
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert account invocation");
    sqlx::query("UPDATE pool_upstream_accounts SET last_activity_at = ?1 WHERE id = ?2")
        .bind("2026-03-11 20:35:00")
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("persist account last activity");

    let Json(response) =
        list_upstream_accounts(State(state), Query(ListUpstreamAccountsQuery::default()))
            .await
            .expect("list upstream accounts");
    let response_json = serde_json::to_value(response).expect("serialize upstream accounts");
    let account = response_json
        .get("items")
        .and_then(serde_json::Value::as_array)
        .expect("items array")
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(account_id))
        .expect("account summary");

    assert_eq!(response_json["page"].as_u64(), Some(1));
    assert_eq!(response_json["pageSize"].as_u64(), Some(20));
    assert_eq!(response_json["total"].as_u64(), Some(1));
    assert_eq!(response_json["metrics"]["total"].as_u64(), Some(1));
    assert_eq!(response_json["metrics"]["oauth"].as_u64(), Some(0));
    assert_eq!(response_json["metrics"]["apiKey"].as_u64(), Some(1));
    assert_eq!(response_json["metrics"]["attention"].as_u64(), Some(0));
    assert_eq!(
        account
            .get("displayStatus")
            .and_then(serde_json::Value::as_str),
        Some("active")
    );
    assert_eq!(
        account
            .get("lastActivityAt")
            .and_then(serde_json::Value::as_str),
        Some("2026-03-11T12:35:00Z")
    );
}

async fn seed_group_tag_roster(state: &Arc<AppState>) -> (i64, i64) {
    let alpha_id = insert_test_pool_oauth_account(state, "Alpha", "upstream-alpha").await;
    set_test_account_group_name(&state.pool, alpha_id, Some("prod")).await;
    let beta_id = insert_test_pool_oauth_account(state, "Beta", "upstream-beta").await;
    set_test_account_group_name(&state.pool, beta_id, Some("production")).await;
    let gamma_id = insert_test_pool_oauth_account(state, "Gamma", "upstream-gamma").await;
    let delta_id = insert_test_pool_oauth_account(state, "Delta", "upstream-delta").await;
    set_test_account_group_name(&state.pool, delta_id, Some("Prod")).await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = NULL WHERE id = ?1")
        .bind(gamma_id)
        .execute(&state.pool)
        .await
        .expect("clear gamma group to simulate legacy ungrouped account");
    let now_iso = format_utc_iso(Utc::now());
    let vip_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (name, allow_cut_out, allow_cut_in, created_at, updated_at)
        VALUES (?1, 1, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("vip")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert vip tag");
    let burst_safe_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (name, allow_cut_out, allow_cut_in, created_at, updated_at)
        VALUES (?1, 1, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("burst-safe")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert burst-safe tag");
    for (account_id, tag_id) in [
        (alpha_id, vip_tag_id),
        (alpha_id, burst_safe_tag_id),
        (beta_id, vip_tag_id),
        (gamma_id, vip_tag_id),
        (gamma_id, burst_safe_tag_id),
        (delta_id, vip_tag_id),
    ] {
        sqlx::query(
            "INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)",
        )
        .bind(account_id)
        .bind(tag_id)
        .bind(&now_iso)
        .execute(&state.pool)
        .await
        .expect("insert account tag link");
    }
    (vip_tag_id, burst_safe_tag_id)
}

async fn query_group_tag_roster(state: &Arc<AppState>, query: ListUpstreamAccountsQuery) -> Value {
    let Json(response) = list_upstream_accounts(State(state.clone()), Query(query))
        .await
        .expect("list filtered upstream accounts");
    serde_json::to_value(response).expect("serialize filtered upstream accounts")
}

async fn assert_group_search_filter(
    state: &Arc<AppState>,
    vip_tag_id: i64,
    burst_safe_tag_id: i64,
) {
    let response = query_group_tag_roster(
        state,
        ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: Vec::new(),
            group_search: Some("prod".to_string()),
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: vec![vip_tag_id, burst_safe_tag_id, vip_tag_id],
        },
    )
    .await;
    let names = response["items"]
        .as_array()
        .expect("filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Alpha"]);
    assert_eq!(response["hasUngroupedAccounts"].as_bool(), Some(true));
}

async fn assert_exact_group_filter(state: &Arc<AppState>, vip_tag_id: i64) {
    let response = query_group_tag_roster(
        state,
        ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: vec!["Prod".to_string()],
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: vec![vip_tag_id],
        },
    )
    .await;
    let names = response["items"]
        .as_array()
        .expect("exact-group filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Delta"]);
}

async fn assert_multi_group_filter(state: &Arc<AppState>, vip_tag_id: i64) {
    let response = query_group_tag_roster(
        state,
        ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: vec!["Prod".to_string(), "prod".to_string()],
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: vec![vip_tag_id],
        },
    )
    .await;
    let names = response["items"]
        .as_array()
        .expect("multi exact-group filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Delta", "Alpha"]);
}

async fn assert_ungrouped_filter(state: &Arc<AppState>, vip_tag_id: i64, burst_safe_tag_id: i64) {
    let response = query_group_tag_roster(
        state,
        ListUpstreamAccountsQuery {
            kind: Some("oauth_codex".to_string()),
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: Some(true),
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: vec![vip_tag_id, burst_safe_tag_id],
        },
    )
    .await;
    let names = response["items"]
        .as_array()
        .expect("ungrouped filtered items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Gamma"]);
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_filters_groups_and_tags_server_side() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let (vip_tag_id, burst_safe_tag_id) = seed_group_tag_roster(&state).await;
    assert_group_search_filter(&state, vip_tag_id, burst_safe_tag_id).await;
    assert_exact_group_filter(&state, vip_tag_id).await;
    assert_multi_group_filter(&state, vip_tag_id).await;
    assert_ungrouped_filter(&state, vip_tag_id, burst_safe_tag_id).await;
}

#[tokio::test]
pub(crate) async fn upstream_account_schema_normalizes_blank_group_names_to_default_group() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_oauth_account(&state, "Legacy Blank Group", "oauth-legacy-blank-group")
            .await;
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = '   ' WHERE id = ?1")
        .bind(account_id)
        .execute(&state.pool)
        .await
        .expect("write legacy blank group");

    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("normalize upstream account schema");

    let group_name: Option<String> =
        sqlx::query_scalar("SELECT group_name FROM pool_upstream_accounts WHERE id = ?1")
            .bind(account_id)
            .fetch_one(&state.pool)
            .await
            .expect("load normalized group");
    assert_eq!(
        group_name.as_deref(),
        Some(DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME)
    );

    let Json(ungrouped_filtered) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: Some(true),
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: None,
            page_size: None,
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list legacy ungrouped compatibility after normalization");
    let ungrouped_filtered_json =
        serde_json::to_value(ungrouped_filtered).expect("serialize normalized ungrouped roster");
    let ungrouped_filtered_names = ungrouped_filtered_json["items"]
        .as_array()
        .expect("normalized ungrouped items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(ungrouped_filtered_names, vec!["Legacy Blank Group"]);
}

async fn seed_display_status_roster(state: &Arc<AppState>) -> (i64, i64, i64) {
    let alpha_id = insert_test_pool_api_key_account(state, "Alpha", "upstream-alpha").await;
    let beta_id = insert_test_pool_api_key_account(state, "Beta", "upstream-beta").await;
    let gamma_id = insert_test_pool_api_key_account(state, "Gamma", "upstream-gamma").await;
    for index in 0..19 {
        let display_name = format!("Extra {index:02}");
        let api_key = format!("upstream-extra-{index:02}");
        insert_test_pool_api_key_account(state, &display_name, &api_key).await;
    }

    let now = Utc::now();
    sqlx::query(
        "UPDATE pool_upstream_accounts SET last_selected_at = ?2, cooldown_until = ?3 WHERE id = ?1",
    )
    .bind(alpha_id)
    .bind(format_test_recent_active_timestamp(now))
    .bind::<Option<String>>(None)
    .execute(&state.pool)
    .await
    .expect("mark alpha working");
    set_test_account_rate_limited_cooldown(&state.pool, beta_id, 600).await;
    sqlx::query("UPDATE pool_upstream_accounts SET enabled = 0 WHERE id = ?1")
        .bind(beta_id)
        .execute(&state.pool)
        .await
        .expect("disable beta account");
    sqlx::query(
        "UPDATE pool_upstream_accounts SET status = ?2, last_error = ?3, last_error_at = ?4, last_route_failure_at = NULL, last_route_failure_kind = NULL WHERE id = ?1",
    )
    .bind(beta_id)
    .bind("syncing")
    .bind("Authentication token has been invalidated, please sign in again")
    .bind(format_utc_iso(now))
    .execute(&state.pool)
    .await
    .expect("seed beta stale disabled syncing state");
    set_test_account_rate_limited_cooldown(&state.pool, gamma_id, 600).await;
    (alpha_id, beta_id, gamma_id)
}

async fn assert_active_accounts_page(state: &Arc<AppState>, alpha_id: i64) {
    let Json(active_page_two) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: Some("active".to_string()),
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(2),
            page_size: Some(20),
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list active upstream accounts page two");
    let active_page_two_json =
        serde_json::to_value(active_page_two).expect("serialize active page two response");
    let active_names = active_page_two_json["items"]
        .as_array()
        .expect("active page items array")
        .iter()
        .filter_map(|item| item.get("displayName").and_then(serde_json::Value::as_str))
        .collect::<Vec<_>>();
    assert_eq!(active_names, vec!["Alpha"]);
    assert_eq!(active_page_two_json["total"].as_u64(), Some(21));
    assert_eq!(active_page_two_json["page"].as_u64(), Some(2));
    assert_eq!(active_page_two_json["pageSize"].as_u64(), Some(20));
    assert_eq!(active_page_two_json["metrics"]["total"].as_u64(), Some(21));
    assert_eq!(active_page_two_json["metrics"]["apiKey"].as_u64(), Some(21));
    assert_eq!(
        active_page_two_json["metrics"]["attention"].as_u64(),
        Some(1)
    );
    assert_eq!(
        active_page_two_json["items"][0]["id"].as_i64(),
        Some(alpha_id)
    );
}

async fn assert_disabled_accounts_page(state: &Arc<AppState>, beta_id: i64) {
    let Json(disabled_only) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: Some("disabled".to_string()),
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list disabled upstream accounts");
    let disabled_only_json =
        serde_json::to_value(disabled_only).expect("serialize disabled response");
    let disabled_items = disabled_only_json["items"]
        .as_array()
        .expect("disabled items array");
    assert_eq!(disabled_only_json["total"].as_u64(), Some(1));
    assert_eq!(disabled_items.len(), 1);
    assert_eq!(
        disabled_items[0]
            .get("id")
            .and_then(serde_json::Value::as_i64),
        Some(beta_id)
    );
    assert_eq!(
        disabled_items[0]
            .get("displayStatus")
            .and_then(serde_json::Value::as_str),
        Some("disabled")
    );
    assert_eq!(
        disabled_items[0]
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(
        disabled_items[0]
            .get("syncState")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );
    assert_eq!(disabled_only_json["metrics"]["attention"].as_u64(), Some(0));
}

async fn assert_split_status_page(state: &Arc<AppState>, gamma_id: i64) {
    let Json(split_status_filtered) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: vec!["rate_limited".to_string()],
            enable_status: vec!["enabled".to_string()],
            health_status: vec!["normal".to_string()],
            page: Some(1),
            page_size: Some(20),
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list split status filtered upstream accounts");
    let split_status_filtered_json =
        serde_json::to_value(split_status_filtered).expect("serialize split status response");
    let split_items = split_status_filtered_json["items"]
        .as_array()
        .expect("split status items array");
    assert_eq!(split_status_filtered_json["total"].as_u64(), Some(1));
    assert_eq!(split_items.len(), 1);
    assert_eq!(
        split_items[0].get("id").and_then(serde_json::Value::as_i64),
        Some(gamma_id)
    );
    assert_eq!(
        split_items[0]
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("rate_limited")
    );
    assert_eq!(
        split_items[0]
            .get("enableStatus")
            .and_then(serde_json::Value::as_str),
        Some("enabled")
    );
    assert_eq!(
        split_items[0]
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(
        split_status_filtered_json["metrics"]["attention"].as_u64(),
        Some(1)
    );
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_filters_by_display_status_and_paginate_server_side() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let (alpha_id, beta_id, gamma_id) = seed_display_status_roster(&state).await;
    assert_active_accounts_page(&state, alpha_id).await;
    assert_disabled_accounts_page(&state, beta_id).await;
    assert_split_status_page(&state, gamma_id).await;
}

async fn seed_abnormal_account_statuses(state: &Arc<AppState>) -> (i64, i64) {
    let reauth_id =
        insert_test_pool_api_key_account(state, "Needs Reauth", "upstream-reauth").await;
    let syncing_id =
        insert_test_pool_api_key_account(state, "Currently Syncing", "upstream-syncing").await;

    let now = Utc::now();
    let now_iso = format_utc_iso(now);
    let cooldown_until = format_utc_iso(now + ChronoDuration::minutes(10));
    let recently_selected = format_test_recent_active_timestamp(now);

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = ?3,
            last_error_at = ?4,
            cooldown_until = ?5,
            last_selected_at = ?6
        WHERE id = ?1
        "#,
    )
    .bind(reauth_id)
    .bind("needs_reauth")
    .bind("refresh token expired")
    .bind(&now_iso)
    .bind(&cooldown_until)
    .bind(&recently_selected)
    .execute(&state.pool)
    .await
    .expect("mark reauth account abnormal");

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET status = ?2,
            last_error = NULL,
            last_error_at = NULL,
            cooldown_until = ?3,
            last_selected_at = ?4
        WHERE id = ?1
        "#,
    )
    .bind(syncing_id)
    .bind("syncing")
    .bind(&cooldown_until)
    .bind(&recently_selected)
    .execute(&state.pool)
    .await
    .expect("mark syncing account in cooldown");
    (reauth_id, syncing_id)
}

async fn assert_abnormal_account_statuses(state: &Arc<AppState>, reauth_id: i64, syncing_id: i64) {
    let Json(response) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list upstream accounts with abnormal states");
    let response_json =
        serde_json::to_value(response).expect("serialize abnormal upstream accounts");
    let items = response_json["items"]
        .as_array()
        .expect("abnormal items array");

    let reauth_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(reauth_id))
        .expect("reauth item present");
    assert_eq!(
        reauth_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("unavailable")
    );
    assert_eq!(
        reauth_item
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("needs_reauth")
    );
    assert_eq!(
        reauth_item
            .get("syncState")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );

    let syncing_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(syncing_id))
        .expect("syncing item present");
    assert_eq!(
        syncing_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );
    assert_eq!(
        syncing_item
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(
        syncing_item
            .get("syncState")
            .and_then(serde_json::Value::as_str),
        Some("syncing")
    );
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_clamps_work_status_for_abnormal_or_syncing_accounts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let (reauth_id, syncing_id) = seed_abnormal_account_statuses(&state).await;
    assert_abnormal_account_statuses(&state, reauth_id, syncing_id).await;
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_work_status_uses_five_minute_activity_window() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let recent_id =
        insert_test_pool_api_key_account(&state, "Recent Working", "upstream-recent-working").await;
    let stale_id =
        insert_test_pool_api_key_account(&state, "Stale Working", "upstream-stale-working").await;

    let now = Utc::now();
    sqlx::query("UPDATE pool_upstream_accounts SET last_selected_at = ?2 WHERE id = ?1")
        .bind(recent_id)
        .bind(format_test_recent_active_timestamp(now))
        .execute(&state.pool)
        .await
        .expect("mark recent working account");
    sqlx::query("UPDATE pool_upstream_accounts SET last_selected_at = ?2 WHERE id = ?1")
        .bind(stale_id)
        .bind(format_test_stale_active_timestamp(now))
        .execute(&state.pool)
        .await
        .expect("mark stale working account");

    let Json(response) =
        list_upstream_accounts(State(state), Query(ListUpstreamAccountsQuery::default()))
            .await
            .expect("list upstream accounts");
    let payload = serde_json::to_value(response).expect("serialize upstream accounts");
    let items = payload["items"].as_array().expect("items array");

    let recent_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(recent_id))
        .expect("recent item present");
    assert_eq!(
        recent_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("working")
    );

    let stale_item = items
        .iter()
        .find(|item| item.get("id").and_then(serde_json::Value::as_i64) == Some(stale_id))
        .expect("stale item present");
    assert_eq!(
        stale_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("idle")
    );
}

#[tokio::test]
pub(crate) async fn upstream_account_summary_and_detail_include_active_conversation_count() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let account_id =
        insert_test_pool_api_key_account(&state, "Sticky conversations", "upstream-sticky").await;

    let now = Utc::now();
    let created_at = format_utc_iso(now - ChronoDuration::minutes(20));
    let first_recent_seen_at = format_utc_iso(now - ChronoDuration::minutes(2));
    let second_recent_seen_at = format_test_recent_active_timestamp(now);
    let stale_seen_at = format_test_stale_active_timestamp(now);

    for (sticky_key, last_seen_at) in [
        ("sticky-recent-1", first_recent_seen_at.as_str()),
        ("sticky-recent-2", second_recent_seen_at.as_str()),
        ("sticky-stale", stale_seen_at.as_str()),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_routes (
                sticky_key, account_id, created_at, updated_at, last_seen_at
            ) VALUES (?1, ?2, ?3, ?3, ?4)
            "#,
        )
        .bind(sticky_key)
        .bind(account_id)
        .bind(&created_at)
        .bind(last_seen_at)
        .execute(&state.pool)
        .await
        .expect("insert sticky route");
    }
    for (sticky_key, model_key) in [
        ("sticky-recent-1", "gpt-5.4"),
        ("sticky-model-only", "gpt-5.1-codex-max"),
    ] {
        sqlx::query(
            r#"
            INSERT INTO pool_sticky_model_routes (
                sticky_key, model_key, account_id, created_at, updated_at, last_seen_at
            ) VALUES (?1, ?2, ?3, ?4, ?4, ?5)
            "#,
        )
        .bind(sticky_key)
        .bind(model_key)
        .bind(account_id)
        .bind(&created_at)
        .bind(&second_recent_seen_at)
        .execute(&state.pool)
        .await
        .expect("insert model sticky route");
    }

    let Json(list_response) = list_upstream_accounts(
        State(state.clone()),
        Query(ListUpstreamAccountsQuery::default()),
    )
    .await
    .expect("list upstream accounts");
    let list_json = serde_json::to_value(list_response).expect("serialize upstream account list");
    let list_item = list_json["items"]
        .as_array()
        .and_then(|items| items.first())
        .expect("list item present");
    assert_eq!(
        list_item
            .get("activeConversationCount")
            .and_then(serde_json::Value::as_i64),
        Some(3)
    );

    let Json(detail_response) = get_upstream_account(
        State(state),
        axum::extract::Path(account_id),
        axum::extract::Query(GetUpstreamAccountQuery::default()),
    )
    .await
    .expect("load upstream account detail");
    let detail_json =
        serde_json::to_value(detail_response).expect("serialize upstream account detail");
    assert_eq!(
        detail_json
            .get("activeConversationCount")
            .and_then(serde_json::Value::as_i64),
        Some(3)
    );
}

#[tokio::test]
pub(crate) async fn list_upstream_accounts_keeps_generic_retry_cooldown_idle() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let generic_cooldown_id =
        insert_test_pool_api_key_account(&state, "Generic Cooldown", "upstream-generic").await;

    set_test_account_generic_route_cooldown(&state.pool, generic_cooldown_id, 600).await;

    let Json(response) = list_upstream_accounts(
        State(state),
        Query(ListUpstreamAccountsQuery {
            kind: None,
            group_exact: Vec::new(),
            group_search: None,
            group_ungrouped: None,
            status: None,
            work_status: Vec::new(),
            enable_status: Vec::new(),
            health_status: Vec::new(),
            page: Some(1),
            page_size: Some(20),
            include_all: None,
            tag_ids: Vec::new(),
        }),
    )
    .await
    .expect("list upstream accounts with generic cooldown");
    let response_json =
        serde_json::to_value(response).expect("serialize generic cooldown upstream accounts");
    let items = response_json["items"]
        .as_array()
        .expect("generic cooldown items array");

    let generic_item = items
        .iter()
        .find(|item| {
            item.get("id").and_then(serde_json::Value::as_i64) == Some(generic_cooldown_id)
        })
        .expect("generic cooldown item present");
    assert_eq!(
        generic_item
            .get("workStatus")
            .and_then(serde_json::Value::as_str),
        Some("degraded")
    );
    assert_eq!(
        generic_item
            .get("healthStatus")
            .and_then(serde_json::Value::as_str),
        Some("normal")
    );
    assert_eq!(response_json["metrics"]["attention"].as_u64(), Some(1));
}

struct RecentActionsFixture {
    account_id: i64,
    event_id: i64,
    invocation_event_id: i64,
}

async fn insert_recent_action_attempt(pool: &SqlitePool, account_id: i64) -> i64 {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_request_attempts (
            invoke_id, occurred_at, endpoint, route_mode, upstream_account_id,
            attempt_index, distinct_account_index, same_account_retry_index,
            status, request_model
        )
        VALUES (?1, ?2, ?3, ?4, ?5, 1, 1, 0, ?6, ?7)
        "#,
    )
    .bind("recent-action-model-impact")
    .bind("2026-06-25 11:30:00")
    .bind("/v1/responses")
    .bind("pool")
    .bind(account_id)
    .bind("failed")
    .bind("gpt-5.6-terra")
    .execute(pool)
    .await
    .expect("insert upstream request attempt")
    .last_insert_rowid()
}

async fn insert_recent_action_account_event(
    pool: &SqlitePool,
    account_id: i64,
    attempt_id: i64,
) -> i64 {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, account_display_name,
            result, result_description, reason_code, attempt_id, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(account_id)
    .bind("2026-06-25 11:30:00")
    .bind("account_updated")
    .bind("manual")
    .bind("Recent actions gated")
    .bind("success")
    .bind("settings saved")
    .bind("account_updated")
    .bind(attempt_id)
    .bind("2026-06-25 11:30:00")
    .execute(pool)
    .await
    .expect("insert upstream account event")
    .last_insert_rowid()
}

async fn insert_recent_action_invocations(pool: &SqlitePool) {
    for (occurred_at, stored_model, request_model) in [
        ("2026-06-25 11:29:00", "gpt-5.4-response", "gpt-5.4-request"),
        (
            "2026-06-25 11:31:00",
            "gpt-5.7-response",
            "gpt-5.7-unrelated",
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, model, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind("reused-invocation-model-impact")
        .bind(occurred_at)
        .bind("proxy")
        .bind("failed")
        .bind(stored_model)
        .bind(json!({ "requestModel": request_model }).to_string())
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation model fallback fixture");
    }
}

async fn insert_recent_action_invocation_event(pool: &SqlitePool, account_id: i64) -> i64 {
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_events (
            account_id, occurred_at, action, source, account_display_name,
            result, result_description, reason_code, invoke_id, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(account_id)
    .bind("2026-06-25T03:29:20Z")
    .bind("route_cooldown_started")
    .bind("call")
    .bind("Recent actions gated")
    .bind("failed")
    .bind("upstream unavailable")
    .bind("upstream_http_5xx")
    .bind("reused-invocation-model-impact")
    .bind("2026-06-25T03:29:20Z")
    .execute(pool)
    .await
    .expect("insert invocation-linked upstream account event")
    .last_insert_rowid()
}

async fn seed_recent_actions_fixture(state: &Arc<AppState>) -> RecentActionsFixture {
    let account_id =
        insert_test_pool_api_key_account(state, "Recent actions gated", "upstream-gated").await;
    let attempt_id = insert_recent_action_attempt(&state.pool, account_id).await;
    let event_id = insert_recent_action_account_event(&state.pool, account_id, attempt_id).await;
    insert_recent_action_invocations(&state.pool).await;
    let invocation_event_id = insert_recent_action_invocation_event(&state.pool, account_id).await;
    RecentActionsFixture {
        account_id,
        event_id,
        invocation_event_id,
    }
}

async fn assert_recent_actions_default_is_empty(state: &Arc<AppState>, account_id: i64) {
    let Json(default_detail) = get_upstream_account(
        State(state.clone()),
        axum::extract::Path(account_id),
        axum::extract::Query(GetUpstreamAccountQuery::default()),
    )
    .await
    .expect("load default upstream account detail");
    let default_detail_json =
        serde_json::to_value(default_detail).expect("serialize default upstream account detail");
    assert!(
        default_detail_json["recentActions"]
            .as_array()
            .is_some_and(|items| items.is_empty()),
        "default detail fetch should skip recent actions for overview first paint"
    );
}

async fn assert_recent_actions_detail(
    state: &Arc<AppState>,
    account_id: i64,
    invocation_event_id: i64,
) {
    let Json(detail) = get_upstream_account(
        State(state.clone()),
        axum::extract::Path(account_id),
        axum::extract::Query(GetUpstreamAccountQuery {
            include_recent_actions: Some(true),
        }),
    )
    .await
    .expect("load upstream account detail with recent actions");
    let detail_json = serde_json::to_value(detail)
        .expect("serialize upstream account detail with recent actions");
    assert!(
        detail_json["recentActions"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default()
            >= 1
    );
    assert_eq!(
        detail_json["recentActions"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("action").and_then(Value::as_str) == Some("account_updated")
                })
            })
            .and_then(|item| item.get("action"))
            .and_then(Value::as_str),
        Some("account_updated")
    );
    assert_eq!(
        detail_json["recentActions"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("action").and_then(Value::as_str) == Some("account_updated")
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(Value::as_str),
        Some("gpt-5.6-terra"),
        "account events must expose the model already captured by their linked attempt"
    );
    assert_eq!(
        detail_json["recentActions"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(Value::as_i64) == Some(invocation_event_id)
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(Value::as_str),
        Some("gpt-5.4-request"),
        "invocation fallback must use the matching request model, not a response or newer call"
    );
}

async fn assert_recent_actions_global_list(
    state: &Arc<AppState>,
    event_id: i64,
    invocation_event_id: i64,
) {
    let Json(event_list) = list_upstream_account_action_events(
        State(state.clone()),
        axum::extract::Query(ListUpstreamAccountActionEventsQuery {
            account: Some("Recent actions gated".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("load global upstream account event list");
    let event_list_json =
        serde_json::to_value(event_list).expect("serialize upstream account event list");
    assert_eq!(
        event_list_json["items"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_i64) == Some(event_id))
            })
            .and_then(|item| item.get("model"))
            .and_then(Value::as_str),
        Some("gpt-5.6-terra"),
        "global account event lists must expose the linked attempt model"
    );
    assert_eq!(
        event_list_json["items"]
            .as_array()
            .and_then(|items| {
                items.iter().find(|item| {
                    item.get("id").and_then(Value::as_i64) == Some(invocation_event_id)
                })
            })
            .and_then(|item| item.get("model"))
            .and_then(Value::as_str),
        Some("gpt-5.4-request"),
        "global event fallback must correlate duplicate invocation ids by occurrence"
    );
}

#[tokio::test]
pub(crate) async fn get_upstream_account_omits_recent_actions_by_default_and_loads_them_on_demand()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let fixture = seed_recent_actions_fixture(&state).await;

    assert_recent_actions_default_is_empty(&state, fixture.account_id).await;
    assert_recent_actions_detail(&state, fixture.account_id, fixture.invocation_event_id).await;
    assert_recent_actions_global_list(&state, fixture.event_id, fixture.invocation_event_id).await;
}

use super::*;
