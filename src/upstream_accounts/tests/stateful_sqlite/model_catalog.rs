use super::*;

#[test]
fn model_catalog_normalization_trims_empty_and_exact_duplicates() {
    assert_eq!(
        normalize_model_ids(vec![
            " gpt-5.5 ".to_string(),
            "".to_string(),
            "gpt-5.5".to_string(),
            "gpt-5.4".to_string(),
        ]),
        vec!["gpt-5.5".to_string(), "gpt-5.4".to_string()]
    );
}

#[tokio::test]
async fn model_catalog_failure_retains_last_successful_snapshot() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Catalog retention").await;
    let first_attempt = "2026-09-05T10:00:00Z";
    persist_model_catalog_success(
        &state.pool,
        account_id,
        &["gpt-5.5".to_string(), "gpt-5.4".to_string()],
        first_attempt,
    )
    .await
    .expect("persist successful catalog");
    persist_model_catalog_failure(
        &state.pool,
        account_id,
        "2026-09-06T10:00:00Z",
        "upstream_http_502",
        "The upstream returned HTTP 502.",
    )
    .await
    .expect("persist failed catalog");

    let catalog = load_upstream_account_model_catalog(&state.pool, account_id)
        .await
        .expect("load retained catalog");
    assert_eq!(catalog.models, vec!["gpt-5.5", "gpt-5.4"]);
    assert_eq!(catalog.status, "failed");
    assert_eq!(catalog.last_successful_at.as_deref(), Some(first_attempt));
    assert_eq!(
        catalog.error.as_ref().map(|error| error.code.as_str()),
        Some("upstream_http_502")
    );
}

#[tokio::test]
async fn model_catalog_success_replaces_only_catalog_snapshot() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let account_id = insert_api_key_account(&state.pool, "Catalog replacement").await;
    let policy_before: Option<String> = sqlx::query_scalar(
        "SELECT policy_available_models_json FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load policy before");
    persist_model_catalog_success(
        &state.pool,
        account_id,
        &[" gpt-5.5 ".to_string(), "gpt-5.5".to_string()],
        "2026-09-06T10:00:00Z",
    )
    .await
    .expect("persist replacement catalog");
    let policy_after: Option<String> = sqlx::query_scalar(
        "SELECT policy_available_models_json FROM pool_upstream_accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load policy after");
    let catalog = load_upstream_account_model_catalog(&state.pool, account_id)
        .await
        .expect("load replacement catalog");
    assert_eq!(catalog.models, vec!["gpt-5.5"]);
    assert_eq!(policy_after, policy_before);
}

#[tokio::test]
async fn model_catalog_snapshot_is_account_scoped_for_oauth_accounts() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    let account_id = insert_syncable_oauth_account(
        &state.pool,
        crypto_key,
        "Catalog OAuth",
        "catalog-oauth@example.com",
        "org_catalog_oauth",
        "user_catalog_oauth",
    )
    .await;
    persist_model_catalog_success(
        &state.pool,
        account_id,
        &["oauth-model".to_string()],
        "2026-09-06T10:00:00Z",
    )
    .await
    .expect("persist OAuth catalog");
    let catalog = load_upstream_account_model_catalog(&state.pool, account_id)
        .await
        .expect("load OAuth catalog");
    assert_eq!(catalog.models, vec!["oauth-model"]);
}
