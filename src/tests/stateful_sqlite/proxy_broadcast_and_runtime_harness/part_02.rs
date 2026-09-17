#[tokio::test]
pub(crate) async fn proxy_model_settings_api_preserves_upstream_429_max_retries_when_field_missing()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(5),
            websocket_enabled: Some(true),
            upstream_websocket_default_enabled: Some(true),
            request_body_logging_enabled: Some(false),
            response_body_logging_enabled: Some(false),
            encrypted_session_owner_routing_enabled: Some(false),
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("put settings should succeed");
    assert_eq!(updated.upstream_429_max_retries, 5);
    assert!(!updated.request_body_logging_enabled);
    assert!(!updated.response_body_logging_enabled);

    let legacy_payload = serde_json::from_value::<ProxyModelSettingsUpdateRequest>(json!({
        "hijackEnabled": true,
        "mergeUpstreamEnabled": false,
        "fastModeRewriteMode": "fill_missing",
        "enabledModels": ["gpt-5.2-codex"],
    }))
    .expect("legacy payload should deserialize");

    let Json(updated) =
        put_proxy_settings(State(state.clone()), HeaderMap::new(), Json(legacy_payload))
            .await
            .expect("legacy payload should not reset upstream429MaxRetries");
    assert_eq!(updated.upstream_429_max_retries, 5);
    assert!(updated.websocket_enabled);
    assert!(updated.upstream_websocket_default_enabled);
    assert!(!updated.request_body_logging_enabled);
    assert!(!updated.response_body_logging_enabled);
    assert!(!updated.encrypted_session_owner_routing_enabled);

    let persisted = load_proxy_model_settings(&state.pool)
        .await
        .expect("settings should persist");
    assert_eq!(persisted.upstream_429_max_retries, 5);
    assert!(persisted.websocket_enabled);
    assert!(persisted.upstream_websocket_default_enabled);
    assert!(!persisted.request_body_logging_enabled);
    assert!(!persisted.response_body_logging_enabled);
    assert!(!persisted.encrypted_session_owner_routing_enabled);
}

#[tokio::test]
pub(crate) async fn proxy_websocket_settings_initialize_from_env_once_then_persist() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let mut config = test_config();
    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    ensure_proxy_websocket_settings_initialized(&pool, &config)
        .await
        .expect("initialize websocket settings");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(settings.websocket_enabled);
    assert!(settings.upstream_websocket_default_enabled);

    let mut next = settings.clone();
    next.websocket_enabled = false;
    next.upstream_websocket_default_enabled = false;
    save_proxy_model_settings(&pool, next)
        .await
        .expect("save user websocket settings");

    config.openai_proxy_websocket_enabled = true;
    config.openai_proxy_upstream_websocket_default_enabled = true;
    ensure_proxy_websocket_settings_initialized(&pool, &config)
        .await
        .expect("second initialization should not override user settings");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("reload proxy model settings");
    assert!(!settings.websocket_enabled);
    assert!(!settings.upstream_websocket_default_enabled);
    assert!(settings.request_body_logging_enabled);
    assert!(settings.response_body_logging_enabled);
}

#[tokio::test]
pub(crate) async fn proxy_encrypted_owner_routing_setting_initialize_from_env_once_then_persist() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let mut config = test_config();
    config.openai_proxy_encrypted_session_owner_routing_enabled = true;
    ensure_proxy_encrypted_session_owner_routing_setting_initialized(&pool, &config)
        .await
        .expect("initialize encrypted owner routing setting");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(settings.encrypted_session_owner_routing_enabled);

    let mut next = settings.clone();
    next.encrypted_session_owner_routing_enabled = false;
    save_proxy_model_settings(&pool, next)
        .await
        .expect("save user encrypted owner routing settings");

    config.openai_proxy_encrypted_session_owner_routing_enabled = true;
    ensure_proxy_encrypted_session_owner_routing_setting_initialized(&pool, &config)
        .await
        .expect("second initialization should not override user settings");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("reload proxy model settings");
    assert!(!settings.encrypted_session_owner_routing_enabled);
}

fn assert_legacy_proxy_model_settings_columns(columns: &[String]) {
    for column in [
        "fast_mode_rewrite_mode",
        "openai_proxy_websocket_enabled",
        "openai_proxy_upstream_websocket_default_enabled",
        "request_body_logging_enabled",
        "response_body_logging_enabled",
        "encrypted_session_owner_routing_enabled",
        "encrypted_session_owner_routing_initialized",
    ] {
        assert!(
            columns.iter().any(|candidate| candidate == column),
            "legacy proxy_model_settings column should remain: {column}"
        );
    }
}

#[tokio::test]
pub(crate) async fn ensure_schema_keeps_legacy_fast_mode_rewrite_mode_column_inert() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy proxy_model_settings table");

    sqlx::query(
        r#"
        INSERT INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            enabled_preset_models_json,
            preset_models_migrated
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(1_i64)
    .bind(0_i64)
    .bind(
        serde_json::to_string(&default_enabled_preset_models())
            .expect("serialize default enabled models"),
    )
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("insert legacy proxy_model_settings row");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert_eq!(
        settings.upstream_429_max_retries,
        DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES
    );
    assert!(!settings.websocket_enabled);
    assert!(!settings.upstream_websocket_default_enabled);
    assert!(!settings.encrypted_session_owner_routing_enabled);
    let columns = sqlx::query("PRAGMA table_info('proxy_model_settings')")
        .fetch_all(&pool)
        .await
        .expect("load proxy_model_settings columns")
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<Vec<_>>();
    assert_legacy_proxy_model_settings_columns(&columns);
}

#[tokio::test]
pub(crate) async fn ensure_schema_legacy_missing_owner_routing_column_can_seed_from_env_once() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy proxy_model_settings table");

    sqlx::query(
        r#"
        INSERT INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            enabled_preset_models_json,
            preset_models_migrated
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(1_i64)
    .bind(0_i64)
    .bind(
        serde_json::to_string(&default_enabled_preset_models())
            .expect("serialize default enabled models"),
    )
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("insert legacy proxy_model_settings row");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");

    let mut config = test_config();
    config.openai_proxy_encrypted_session_owner_routing_enabled = true;
    ensure_proxy_encrypted_session_owner_routing_setting_initialized(&pool, &config)
        .await
        .expect("seed encrypted owner routing from env");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after seed");
    assert!(settings.encrypted_session_owner_routing_enabled);

    let initialized = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT encrypted_session_owner_routing_initialized
        FROM proxy_model_settings
        WHERE id = ?1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_one(&pool)
    .await
    .expect("load encrypted owner routing initialization flag");
    assert_eq!(initialized, 1);
}

#[tokio::test]
pub(crate) async fn ensure_schema_preserves_existing_owner_routing_setting_when_column_already_exists()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            encrypted_session_owner_routing_enabled INTEGER NOT NULL DEFAULT 1,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy proxy_model_settings table with owner routing column");

    sqlx::query(
        r#"
        INSERT INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            encrypted_session_owner_routing_enabled,
            enabled_preset_models_json,
            preset_models_migrated
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(1_i64)
    .bind(0_i64)
    .bind(1_i64)
    .bind(
        serde_json::to_string(&default_enabled_preset_models())
            .expect("serialize default enabled models"),
    )
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("insert legacy proxy_model_settings row with owner routing enabled");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");

    let mut config = test_config();
    config.openai_proxy_encrypted_session_owner_routing_enabled = false;
    ensure_proxy_encrypted_session_owner_routing_setting_initialized(&pool, &config)
        .await
        .expect("legacy setting should be marked initialized and preserved");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load preserved proxy model settings");
    assert!(settings.encrypted_session_owner_routing_enabled);

    let initialized = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT encrypted_session_owner_routing_initialized
        FROM proxy_model_settings
        WHERE id = ?1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_one(&pool)
    .await
    .expect("load encrypted owner routing initialization flag");
    assert_eq!(initialized, 1);
}

#[tokio::test]
pub(crate) async fn ensure_schema_appends_new_proxy_models_when_enabled_list_matches_legacy_default()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.4".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.4-pro".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.5".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.5-pro".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-sol".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-terra".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-luna".to_string())
    );
}

#[tokio::test]
pub(crate) async fn ensure_schema_orders_default_proxy_models_newest_first() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");

    assert_eq!(
        settings.enabled_preset_models,
        vec![
            "gpt-5.6-sol".to_string(),
            "gpt-5.6-terra".to_string(),
            "gpt-5.6-luna".to_string(),
            "gpt-5.5".to_string(),
            "gpt-5.5-pro".to_string(),
            "gpt-5.4".to_string(),
            "gpt-5.4-pro".to_string(),
            "gpt-5.3-codex".to_string(),
            "gpt-5.2".to_string(),
            "gpt-5.2-codex".to_string(),
            "gpt-5.1-codex-max".to_string(),
            "gpt-5.1-codex-mini".to_string(),
        ]
    );
}

#[tokio::test]
pub(crate) async fn ensure_schema_appends_latest_proxy_models_when_enabled_list_matches_previous_default()
 {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let previous_enabled = PREVIOUS_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let previous_enabled_json =
        serde_json::to_string(&previous_enabled).expect("serialize previous enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(previous_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force previous enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.5".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.5-pro".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-sol".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-terra".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-luna".to_string())
    );
}

#[tokio::test]
pub(crate) async fn ensure_schema_does_not_append_new_proxy_models_when_enabled_list_is_custom() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let custom_enabled = vec!["gpt-5.2-codex".to_string()];
    let custom_enabled_json =
        serde_json::to_string(&custom_enabled).expect("serialize custom enabled list");
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(custom_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force custom enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert_eq!(settings.enabled_preset_models, custom_enabled);
}

#[tokio::test]
pub(crate) async fn ensure_schema_allows_opting_out_of_new_proxy_models_after_migration() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let normalized_legacy_enabled = normalize_enabled_preset_models(legacy_enabled.clone());
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(&legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");
    let migrated = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after migration");
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.4".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.4-pro".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.5".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.5-pro".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.6-sol".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.6-terra".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.6-luna".to_string())
    );

    // User explicitly removes the new models after migration; schema re-run should not
    // force them back in.
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 2
        WHERE id = ?2
        "#,
    )
    .bind(&legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models after migration");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after opt-out");
    assert_eq!(settings.enabled_preset_models, normalized_legacy_enabled);
}

#[tokio::test]
pub(crate) async fn ensure_schema_reruns_proxy_preset_migration_for_previous_migration_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 1
        WHERE id = ?2
        "#,
    )
    .bind(legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force previous proxy preset migration version");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after rerun");
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-sol".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-terra".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.6-luna".to_string())
    );

    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT preset_models_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_one(&pool)
    .await
    .expect("read migration version");
    assert_eq!(migrated, 2);
}

#[tokio::test]
pub(crate) async fn ensure_schema_marks_proxy_preset_models_migrated_when_enabled_list_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind("[]")
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force empty enabled preset models list");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings.enabled_preset_models.is_empty(),
        "empty enabled list should be preserved"
    );

    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT preset_models_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_one(&pool)
    .await
    .expect("read migration flag");
    assert_eq!(migrated, 2);
}

#[tokio::test]
pub(crate) async fn proxy_model_settings_api_rejects_cross_origin_writes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );

    let err = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect_err("cross-origin write should be rejected");

    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(crate) async fn proxy_model_settings_api_rejects_cross_site_request() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );

    let err = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect_err("cross-site request should be rejected");

    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
pub(crate) async fn proxy_model_settings_api_allows_loopback_proxy_origin_mismatch() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:60080"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: Some(DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES),
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("loopback proxied write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

use super::*;
