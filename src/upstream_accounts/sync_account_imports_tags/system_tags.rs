use super::*;
pub(crate) const GPT55_UNSUPPORTED_SYSTEM_TAG_KEY: &str = "unsupported_model:gpt-5.5";
pub(crate) const GPT55_UNSUPPORTED_SYSTEM_TAG_NAME: &str = "不支持 gpt-5.5";
pub(crate) const UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS: &str = r#"
    id, kind, provider, display_name, group_name, is_mother, note, status, enabled, runtime_routing_generation, email,
    verified_email,
    chatgpt_account_id, chatgpt_user_id, plan_type, plan_type_observed_at, masked_api_key,
    encrypted_credentials, has_refresh_token, token_expires_at,
    access_token_expiry_source, access_token_expiry_observed_at,
    refresh_token_expires_at, refresh_token_expiry_source, refresh_token_expiry_state,
    refresh_token_expiry_observed_at, last_refreshed_at,
    last_synced_at, last_successful_sync_at, last_activity_at, last_error, last_error_at,
    last_action, last_action_source, last_action_reason_code, last_action_reason_message,
    last_action_http_status, last_action_invoke_id, last_action_at,
    last_selected_at, last_route_failure_at, last_route_failure_kind, cooldown_until,
    consecutive_route_failures, temporary_route_failure_streak_started_at,
    compact_support_status, compact_support_observed_at,
    compact_support_reason,
    response_endpoint_capability, response_endpoint_capability_observed_at,
    response_endpoint_capability_reason, policy_response_endpoint_capability_override,
    chat_completions_capability, chat_completions_capability_observed_at,
    chat_completions_capability_reason, policy_chat_completions_capability_override,
    image_endpoint_capability, image_endpoint_capability_observed_at,
    image_endpoint_capability_reason, policy_image_endpoint_capability_override,
    response_image_tool_capability, response_image_tool_capability_observed_at,
    response_image_tool_capability_reason, policy_response_image_tool_capability_override,
    codex_imagegen_capability, codex_imagegen_capability_observed_at,
    codex_imagegen_capability_reason, policy_codex_imagegen_capability_override,
    standalone_search_capability, standalone_search_capability_observed_at,
    standalone_search_capability_reason, policy_standalone_search_capability_override,
    local_primary_limit, local_secondary_limit, local_limit_unit,
    policy_allow_cut_out, policy_allow_cut_in, policy_priority_tier,
    policy_fast_mode_rewrite_mode, policy_image_tool_rewrite_mode,
    policy_codex_imagegen_rewrite_mode,
    policy_request_compression_algorithm, policy_oauth_request_compression_algorithm,
    policy_concurrency_limit,
    policy_upstream_429_retry_enabled, policy_upstream_429_max_retries,
    policy_available_models_json,
    policy_available_models_mode,
    policy_status_change_upstream_http_401,
    policy_status_change_upstream_http_402,
    policy_status_change_upstream_http_403,
    policy_status_change_reauth_required,
    policy_status_change_upstream_http_429_rate_limit,
    policy_status_change_upstream_http_429_quota_exhausted,
    policy_status_change_usage_snapshot_exhausted,
    policy_status_change_quota_still_exhausted,
    policy_status_change_transport_failure,
    policy_status_change_upstream_server_overloaded,
    policy_status_change_upstream_http_5xx,
    policy_responses_first_byte_timeout_secs, policy_compact_first_byte_timeout_secs,
    policy_image_first_byte_timeout_secs,
    policy_responses_stream_timeout_secs, policy_compact_stream_timeout_secs,
    bound_proxy_keys_json,
    upstream_base_url, model_mappings_json, external_client_id, external_source_account_id,
    created_at, updated_at
"#;

pub(crate) fn unsupported_model_system_tag_key(model: &str) -> Option<String> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }
    Some(format!("unsupported_model:{normalized}"))
}

pub(crate) fn unsupported_model_system_tag_name(model: &str) -> String {
    format!("不支持 {model}")
}

pub(crate) async fn ensure_protected_system_tag(
    pool: &Pool<Sqlite>,
    name: &str,
    system_key: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    ensure_protected_system_tag_conn(tx.as_mut(), name, system_key).await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn ensure_protected_system_tag_conn(
    conn: &mut SqliteConnection,
    name: &str,
    system_key: &str,
) -> Result<()> {
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_tags
        SET system_key = ?2,
            protected = 1,
            updated_at = ?3
        WHERE name = ?1
          AND system_key IS NULL
          AND NOT EXISTS (
              SELECT 1 FROM pool_tags existing WHERE existing.system_key = ?2
          )
        "#,
    )
    .bind(name)
    .bind(system_key)
    .bind(&now_iso)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pool_tags (
            name, system_key, protected,
            allow_cut_out, allow_cut_in, priority_tier, fast_mode_rewrite_mode, concurrency_limit,
            upstream_429_retry_enabled, upstream_429_max_retries,
            created_at, updated_at
        )
        VALUES (?1, ?2, 1, 1, 1, 'normal', 'keep_original', 0, 0, 0, ?3, ?3)
        "#,
    )
    .bind(name)
    .bind(system_key)
    .bind(&now_iso)
    .execute(&mut *conn)
    .await?;

    sqlx::query(
        r#"
        UPDATE pool_tags
        SET name = ?1,
            protected = 1,
            updated_at = ?3
        WHERE system_key = ?2
        "#,
    )
    .bind(name)
    .bind(system_key)
    .bind(&now_iso)
    .execute(&mut *conn)
    .await?;

    // Protected system tags are immutable signals. Keep any legacy model list,
    // but discard editable routing fields left on a tag promoted by name.
    sqlx::query(
        r#"
        UPDATE pool_tags
        SET allow_cut_out = 1,
            allow_cut_in = 1,
            priority_tier = 'normal',
            fast_mode_rewrite_mode = 'keep_original',
            concurrency_limit = 0,
            upstream_429_retry_enabled = 0,
            upstream_429_max_retries = 0,
            updated_at = ?2
        WHERE system_key = ?1
        "#,
    )
    .bind(system_key)
    .bind(&now_iso)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

pub(crate) async fn ensure_gpt55_unsupported_system_tag(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_protected_system_tag(
        pool,
        GPT55_UNSUPPORTED_SYSTEM_TAG_NAME,
        GPT55_UNSUPPORTED_SYSTEM_TAG_KEY,
    )
    .await
}

pub(crate) async fn cleanup_non_system_tags(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(
        r#"
        UPDATE pool_oauth_login_sessions
        SET tag_ids_json = (
            SELECT CASE
                WHEN COUNT(*) = 0 THEN NULL
                ELSE json_group_array(
                    CASE
                        WHEN json_each.type IN ('object', 'array') THEN json(json_each.value)
                        WHEN json_each.type IN ('true', 'false') THEN json(json_each.type)
                        ELSE json_each.value
                    END
                )
            END
            FROM json_each(pool_oauth_login_sessions.tag_ids_json)
            WHERE json_each.type != 'integer'
               OR json_each.value NOT IN (
                SELECT id
                FROM pool_tags
                WHERE system_key IS NULL
            )
        )
        WHERE json_valid(tag_ids_json)
          AND json_type(tag_ids_json) = 'array'
          AND EXISTS (
              SELECT 1
              FROM json_each(pool_oauth_login_sessions.tag_ids_json)
              WHERE json_each.type = 'integer'
                AND json_each.value IN (
                    SELECT id
                    FROM pool_tags
                    WHERE system_key IS NULL
                )
          )
        "#,
    )
    .execute(tx.as_mut())
    .await?;
    sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_tags
        WHERE tag_id IN (
            SELECT id
            FROM pool_tags
            WHERE system_key IS NULL
        )
        "#,
    )
    .execute(tx.as_mut())
    .await?;
    sqlx::query(
        r#"
        DELETE FROM pool_tags
        WHERE system_key IS NULL
        "#,
    )
    .execute(tx.as_mut())
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) fn reject_manual_tag_ids(tag_ids: &[i64]) -> Result<(), (StatusCode, String)> {
    if tag_ids.is_empty() {
        return Ok(());
    }
    Err((
        StatusCode::BAD_REQUEST,
        "manual tag assignment is no longer supported; omit tagIds".to_string(),
    ))
}

pub(crate) async fn ensure_account_has_unsupported_model_tag(
    pool: &Pool<Sqlite>,
    account_id: i64,
    model: &str,
) -> Result<()> {
    let Some(system_key) = unsupported_model_system_tag_key(model) else {
        return Ok(());
    };
    ensure_protected_system_tag(
        pool,
        &unsupported_model_system_tag_name(model.trim()),
        &system_key,
    )
    .await?;
    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at)
        SELECT ?1, tag.id, ?3, ?3
        FROM pool_tags tag
        WHERE tag.system_key = ?2
        "#,
    )
    .bind(account_id)
    .bind(system_key)
    .bind(&now_iso)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn ensure_account_has_gpt55_unsupported_tag(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<()> {
    ensure_account_has_unsupported_model_tag(pool, account_id, "gpt-5.5").await
}

pub(crate) async fn account_has_gpt55_unsupported_tag(
    pool: &Pool<Sqlite>,
    account_id: i64,
) -> Result<bool> {
    account_has_system_tag(pool, account_id, GPT55_UNSUPPORTED_SYSTEM_TAG_KEY).await
}

pub(crate) async fn account_has_system_tag(
    pool: &Pool<Sqlite>,
    account_id: i64,
    system_key: &str,
) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_account_tags link
        INNER JOIN pool_tags tag ON tag.id = link.tag_id
        WHERE link.account_id = ?1
          AND tag.system_key = ?2
        "#,
    )
    .bind(account_id)
    .bind(system_key)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}
