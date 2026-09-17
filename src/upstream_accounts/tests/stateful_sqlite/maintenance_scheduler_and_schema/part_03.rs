use super::*;
use serde_json::json;

#[tokio::test]
pub(crate) async fn ensure_upstream_accounts_schema_upgrades_legacy_pool_routing_settings_before_seed()
 {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    seed_legacy_pool_routing_settings(&pool).await;

    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("upgrade schema");
    assert_upgraded_pool_routing_schema(&pool).await;
    assert_seeded_pool_routing_settings(&pool).await;
}

async fn seed_legacy_pool_routing_settings(pool: &SqlitePool) {
    sqlx::query(
        r#"
            CREATE TABLE pool_routing_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                encrypted_api_key TEXT,
                masked_api_key TEXT,
                primary_sync_interval_secs INTEGER,
                secondary_sync_interval_secs INTEGER,
                priority_available_account_cap INTEGER,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            )
            "#,
    )
    .execute(pool)
    .await
    .expect("create legacy pool_routing_settings");
    sqlx::query(
        r#"
            CREATE TABLE pool_upstream_account_model_routes (
                account_id INTEGER NOT NULL,
                model TEXT NOT NULL,
                state TEXT NOT NULL,
                priority TEXT NOT NULL,
                consecutive_failures INTEGER NOT NULL DEFAULT 0,
                streak_started_at TEXT,
                changed_at TEXT,
                last_seen_at TEXT NOT NULL,
                last_success_at TEXT,
                last_failure_at TEXT,
                last_failure_kind TEXT,
                last_failure_message TEXT,
                cooldown_until TEXT,
                UNIQUE(account_id, model)
            )
            "#,
    )
    .execute(pool)
    .await
    .expect("create legacy model routing table");
    sqlx::query(
        "INSERT INTO pool_upstream_account_model_routes (account_id, model, state, priority, last_seen_at) VALUES (7, 'gpt-legacy', 'degraded', 'demoted', datetime('now'))",
    )
    .execute(pool)
    .await
    .expect("insert legacy model routing row");
    sqlx::query(
        r#"
            INSERT INTO pool_routing_settings (
                id, encrypted_api_key, masked_api_key, primary_sync_interval_secs,
                secondary_sync_interval_secs, priority_available_account_cap, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
            "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .bind("legacy-ciphertext")
    .bind("sk-legacy")
    .bind(300_i64)
    .bind(2400_i64)
    .bind(99_i64)
    .execute(pool)
    .await
    .expect("insert legacy pool routing row");
}

async fn assert_upgraded_pool_routing_schema(pool: &SqlitePool) {
    let columns = sqlx::query("PRAGMA table_info('pool_routing_settings')")
        .fetch_all(pool)
        .await
        .expect("load table info")
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<std::collections::HashSet<_>>();
    for column in [
        "responses_first_byte_timeout_secs",
        "compact_first_byte_timeout_secs",
        "responses_stream_timeout_secs",
        "compact_stream_timeout_secs",
        "default_first_byte_timeout_secs",
        "upstream_handshake_timeout_secs",
        "request_read_timeout_secs",
        "cache_hit_protection_enabled",
        "cache_hit_low_rate_threshold_percent",
        "cache_hit_overflow_mode",
    ] {
        assert!(
            columns.contains(column),
            "expected upgraded schema to contain {column}"
        );
    }
    let model_route_columns =
        sqlx::query("PRAGMA table_info('pool_upstream_account_model_routes')")
            .fetch_all(pool)
            .await
            .expect("load model routing table info")
            .into_iter()
            .filter_map(|row| row.try_get::<String, _>("name").ok())
            .collect::<std::collections::HashSet<_>>();
    for column in [
        "reset_fence_at",
        "cache_concurrency_limit",
        "cache_recovery_limit",
        "cache_low_hit_streak",
        "cache_cooldown_level",
        "cache_last_hit_rate_percent",
    ] {
        assert!(
            model_route_columns.contains(column),
            "expected upgraded model routing schema to contain {column}"
        );
    }
    let persisted_model_route = sqlx::query_as::<_, (String, String, Option<i64>, Option<i64>, i64, i64, Option<i64>)>(
        "SELECT state, priority, cache_concurrency_limit, cache_recovery_limit, cache_low_hit_streak, cache_cooldown_level, cache_last_hit_rate_percent FROM pool_upstream_account_model_routes WHERE account_id = 7 AND model = 'gpt-legacy'",
    )
    .fetch_one(pool)
    .await
    .expect("load upgraded legacy model route");
    assert_eq!(
        persisted_model_route,
        (
            "degraded".to_string(),
            "demoted".to_string(),
            None,
            None,
            0,
            0,
            None
        )
    );
}

async fn assert_seeded_pool_routing_settings(pool: &SqlitePool) {
    let config = usage_snapshot_test_config("http://127.0.0.1:9", "codex-vibe-monitor/test");
    let row = load_pool_routing_settings_seeded(pool, &config)
        .await
        .expect("load upgraded routing settings");
    assert_eq!(row.encrypted_api_key.as_deref(), Some("legacy-ciphertext"));
    assert_eq!(row.masked_api_key.as_deref(), Some("sk-legacy"));
    assert_eq!(row.primary_sync_interval_secs, Some(300));
    assert_eq!(row.secondary_sync_interval_secs, Some(2400));
    assert_eq!(row.priority_available_account_cap, Some(99));
    assert_eq!(row.responses_first_byte_timeout_secs, None);
    assert_eq!(row.compact_first_byte_timeout_secs, None);
    assert_eq!(row.responses_stream_timeout_secs, None);
    assert_eq!(row.compact_stream_timeout_secs, None);
    assert_eq!(row.default_first_byte_timeout_secs, None);
    assert_eq!(row.upstream_handshake_timeout_secs, None);
    assert_eq!(row.request_read_timeout_secs, None);
    assert_eq!(row.cache_hit_protection_enabled, Some(0));
    assert_eq!(row.cache_hit_low_rate_threshold_percent, Some(10));
    assert_eq!(row.cache_hit_overflow_mode.as_deref(), Some("queue"));

    let resolved = resolve_pool_routing_timeouts(pool, &config)
        .await
        .expect("resolve routing timeouts");
    let defaults = pool_routing_timeouts_from_config(&config);
    assert_eq!(
        resolved.responses_first_byte_timeout,
        defaults.responses_first_byte_timeout
    );
    assert_eq!(
        resolved.compact_first_byte_timeout,
        defaults.compact_first_byte_timeout
    );
    assert_eq!(
        resolved.responses_stream_timeout,
        defaults.responses_stream_timeout
    );
    assert_eq!(
        resolved.compact_stream_timeout,
        defaults.compact_stream_timeout
    );
    assert_eq!(
        resolved.default_first_byte_timeout,
        defaults.default_first_byte_timeout
    );
    assert_eq!(resolved.default_send_timeout, defaults.default_send_timeout);
    assert_eq!(resolved.request_read_timeout, defaults.request_read_timeout);
}

#[tokio::test]
pub(crate) async fn ensure_upstream_accounts_schema_resets_mixed_response_capability_once() {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("ensure schema");

    let account_id = insert_oauth_account(&pool, "Legacy Mixed Capability").await;
    seed_mixed_response_capability(&pool, account_id).await;

    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("run capability-axis cutover");

    assert_capability_axis_was_reset(&pool, account_id).await;

    seed_post_cutover_capability(&pool, account_id).await;

    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("rerun schema maintenance after cutover");

    assert_capability_axis_is_stable(&pool, account_id).await;
}

async fn seed_mixed_response_capability(pool: &SqlitePool, account_id: i64) {
    sqlx::query(
        "UPDATE pool_upstream_accounts SET response_endpoint_capability = 'supported', response_endpoint_capability_observed_at = '2026-07-17T08:00:00Z', response_endpoint_capability_reason = 'legacy mixed response capability', policy_response_endpoint_capability_override = 'unsupported' WHERE id = ?1",
    )
    .bind(account_id)
    .execute(pool)
    .await
    .expect("seed legacy mixed response capability");
    sqlx::query(
        "UPDATE pool_routing_settings SET capability_axis_split_migrated = 0 WHERE id = ?1",
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .expect("reset capability-axis migration flag");
}

async fn assert_capability_axis_was_reset(pool: &SqlitePool, account_id: i64) {
    let row = load_upstream_account_row(pool, account_id)
        .await
        .expect("load row after capability-axis cutover")
        .expect("row exists after capability-axis cutover");
    assert_eq!(row.response_endpoint_capability.as_deref(), Some("unknown"));
    assert_eq!(row.chat_completions_capability.as_deref(), Some("unknown"));
    assert_eq!(row.response_endpoint_capability_observed_at, None);
    assert_eq!(row.chat_completions_capability_observed_at, None);
    assert_eq!(row.response_endpoint_capability_reason, None);
    assert_eq!(row.chat_completions_capability_reason, None);
    assert_eq!(row.policy_response_endpoint_capability_override, None);
    assert_eq!(row.policy_chat_completions_capability_override, None);
}

async fn seed_post_cutover_capability(pool: &SqlitePool, account_id: i64) {
    sqlx::query(
        "UPDATE pool_upstream_accounts SET response_endpoint_capability = 'supported', response_endpoint_capability_reason = 'responses endpoint request succeeded', policy_response_endpoint_capability_override = 'supported', chat_completions_capability = 'supported', chat_completions_capability_reason = 'chat completions endpoint request succeeded', policy_chat_completions_capability_override = 'unsupported' WHERE id = ?1",
    )
    .bind(account_id)
    .execute(pool)
    .await
    .expect("seed post-cutover capability state");
}

async fn assert_capability_axis_is_stable(pool: &SqlitePool, account_id: i64) {
    let row = load_upstream_account_row(pool, account_id)
        .await
        .expect("load row after second schema run")
        .expect("row exists after second schema run");
    assert_eq!(
        row.response_endpoint_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        row.chat_completions_capability.as_deref(),
        Some("supported")
    );
    assert_eq!(
        row.policy_response_endpoint_capability_override.as_deref(),
        Some("supported")
    );
    assert_eq!(
        row.policy_chat_completions_capability_override.as_deref(),
        Some("unsupported")
    );
    let migrated: i64 = sqlx::query_scalar(
        "SELECT capability_axis_split_migrated FROM pool_routing_settings WHERE id = ?1",
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .fetch_one(pool)
    .await
    .expect("load capability-axis migration flag");
    assert_eq!(migrated, 1);
}

#[tokio::test]
pub(crate) async fn ensure_upstream_accounts_schema_repairs_only_responses_lite_image_tool_misclassifications()
 {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("ensure schema");

    let repaired_account_id = insert_oauth_account(&pool, "Responses Lite misclassification").await;
    let retained_account_id = insert_oauth_account(&pool, "Actual unsupported image tool").await;
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET response_image_tool_capability = 'unsupported',
            response_image_tool_capability_observed_at = '2026-07-24T00:00:00Z',
            response_image_tool_capability_reason = ?2,
            policy_response_image_tool_capability_override = 'supported'
        WHERE id = ?1
        "#,
    )
    .bind(repaired_account_id)
    .bind("Responses Lite rejected top-level tool type image_generation")
    .execute(&pool)
    .await
    .expect("seed Lite misclassification");
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET response_image_tool_capability = 'unsupported',
            response_image_tool_capability_observed_at = '2026-07-24T00:00:00Z',
            response_image_tool_capability_reason = 'unsupported tool: image_generation',
            policy_response_image_tool_capability_override = 'unsupported'
        WHERE id = ?1
        "#,
    )
    .bind(retained_account_id)
    .execute(&pool)
    .await
    .expect("seed actual unsupported capability");

    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("repair Lite misclassification");

    let repaired = load_upstream_account_row(&pool, repaired_account_id)
        .await
        .expect("load repaired account")
        .expect("repaired account exists");
    assert_eq!(
        repaired.response_image_tool_capability.as_deref(),
        Some("unknown")
    );
    assert_eq!(repaired.response_image_tool_capability_observed_at, None);
    assert_eq!(repaired.response_image_tool_capability_reason, None);
    assert_eq!(
        repaired
            .policy_response_image_tool_capability_override
            .as_deref(),
        Some("supported")
    );

    let retained = load_upstream_account_row(&pool, retained_account_id)
        .await
        .expect("load retained account")
        .expect("retained account exists");
    assert_eq!(
        retained.response_image_tool_capability.as_deref(),
        Some("unsupported")
    );
    assert_eq!(
        retained.response_image_tool_capability_reason.as_deref(),
        Some("unsupported tool: image_generation")
    );
    assert_eq!(
        retained
            .policy_response_image_tool_capability_override
            .as_deref(),
        Some("unsupported")
    );
}

#[tokio::test]
pub(crate) async fn ensure_upstream_accounts_schema_migrates_legacy_block_policy_to_no_new_priority()
 {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite");
    create_legacy_priority_schema(&pool).await;

    ensure_upstream_accounts_schema(&pool)
        .await
        .expect("upgrade legacy policy columns");
    assert_legacy_priority_migration(&pool).await;
}

async fn create_legacy_priority_schema(pool: &SqlitePool) {
    sqlx::query(
        "CREATE TABLE pool_upstream_accounts (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, provider TEXT NOT NULL DEFAULT 'codex', display_name TEXT NOT NULL, status TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1, email TEXT, chatgpt_account_id TEXT, last_synced_at TEXT, last_successful_sync_at TEXT, policy_block_new_conversations INTEGER, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    )
    .execute(pool)
    .await
    .expect("create legacy account table");
    sqlx::query(
        "INSERT INTO pool_upstream_accounts (kind, display_name, status, policy_block_new_conversations, created_at, updated_at) VALUES ('api_key', 'legacy-block', 'active', 1, datetime('now'), datetime('now')), ('api_key', 'legacy-allow', 'active', 0, datetime('now'), datetime('now')), ('api_key', 'legacy-inherit', 'active', NULL, datetime('now'), datetime('now'))",
    )
    .execute(pool)
    .await
    .expect("insert legacy account policies");
    sqlx::query(
        "CREATE TABLE pool_upstream_account_group_notes (group_name TEXT PRIMARY KEY, note TEXT NOT NULL, policy_block_new_conversations INTEGER, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)",
    )
    .execute(pool)
    .await
    .expect("create legacy group table");
    sqlx::query(
        "INSERT INTO pool_upstream_account_group_notes (group_name, note, policy_block_new_conversations, created_at, updated_at) VALUES ('legacy-block-group', '', 1, datetime('now'), datetime('now')), ('legacy-allow-group', '', 0, datetime('now'), datetime('now')), ('legacy-inherit-group', '', NULL, datetime('now'), datetime('now'))",
    )
    .execute(pool)
    .await
    .expect("insert legacy group policies");
}

async fn assert_legacy_priority_migration(pool: &SqlitePool) {
    let legacy_model_mappings = sqlx::query_as::<_, (String, String)>(
        "SELECT display_name, model_mappings_json FROM pool_upstream_accounts ORDER BY display_name",
    )
    .fetch_all(pool)
    .await
    .expect("load migrated model mappings");
    assert_eq!(
        legacy_model_mappings,
        vec![
            ("legacy-allow".to_string(), "[]".to_string()),
            ("legacy-block".to_string(), "[]".to_string()),
            ("legacy-inherit".to_string(), "[]".to_string()),
        ]
    );
    let account_values = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT display_name, policy_priority_tier FROM pool_upstream_accounts ORDER BY display_name",
    )
    .fetch_all(pool)
    .await
    .expect("load account policies");
    assert_eq!(
        account_values,
        vec![
            ("legacy-allow".to_string(), None),
            ("legacy-block".to_string(), Some("no_new".to_string())),
            ("legacy-inherit".to_string(), None),
        ]
    );
    let group_values = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT group_name, policy_priority_tier FROM pool_upstream_account_group_notes ORDER BY group_name",
    )
    .fetch_all(pool)
    .await
    .expect("load group policies");
    assert_eq!(
        group_values,
        vec![
            ("legacy-allow-group".to_string(), None),
            ("legacy-block-group".to_string(), Some("no_new".to_string())),
            ("legacy-inherit-group".to_string(), None),
        ]
    );
}

#[tokio::test]
pub(crate) async fn update_pool_routing_settings_allows_maintenance_only_patch() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    save_pool_routing_api_key(&state.pool, crypto_key, "pool-live-key")
        .await
        .expect("seed pool api key");

    let payload: UpdatePoolRoutingSettingsRequest = serde_json::from_value(json!({
        "maintenance": {
            "secondarySyncIntervalSecs": 2400
        }
    }))
    .expect("deserialize maintenance patch");
    let Json(response) =
        update_pool_routing_settings(State(state.clone()), HeaderMap::new(), Json(payload))
            .await
            .expect("update routing settings");
    let expected_mask = mask_api_key("pool-live-key");

    assert!(response.api_key_configured);
    assert_eq!(
        response.masked_api_key.as_deref(),
        Some(expected_mask.as_str())
    );
    assert_eq!(response.maintenance.primary_sync_interval_secs, 300);
    assert_eq!(response.maintenance.secondary_sync_interval_secs, 2400);
    assert_eq!(response.maintenance.priority_available_account_cap, 100);

    let stored = load_pool_routing_settings(&state.pool)
        .await
        .expect("load routing settings");
    assert!(stored.encrypted_api_key.is_some());
    assert_eq!(stored.secondary_sync_interval_secs, Some(2400));
}

#[tokio::test]
pub(crate) async fn warm_pool_routing_runtime_cache_best_effort_skips_invalid_encrypted_api_key() {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    sqlx::query(
        r#"
            UPDATE pool_routing_settings
            SET encrypted_api_key = ?1
            WHERE id = ?2
            "#,
    )
    .bind("not-a-valid-ciphertext")
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(&state.pool)
    .await
    .expect("poison encrypted api key");

    {
        let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
        *runtime_cache = None;
    }

    assert!(
        refresh_pool_routing_runtime_cache(state.as_ref())
            .await
            .is_err(),
        "invalid ciphertext should still fail direct refresh"
    );

    warm_pool_routing_runtime_cache_best_effort(state.as_ref()).await;

    assert!(
        state.pool_routing_runtime_cache.lock().await.is_none(),
        "best-effort startup warmup should leave the cache empty after decrypt failures"
    );
}

#[tokio::test]
pub(crate) async fn refresh_pool_routing_runtime_cache_preserves_last_good_cache_after_decrypt_failure()
 {
    let state = test_app_state_with_usage_base("http://127.0.0.1:9").await;
    let crypto_key = state
        .upstream_accounts
        .crypto_key
        .as_ref()
        .expect("test crypto key");
    save_pool_routing_api_key(&state.pool, crypto_key, "pool-live-key")
        .await
        .expect("seed pool api key");

    let cache = refresh_pool_routing_runtime_cache(state.as_ref())
        .await
        .expect("populate runtime cache");
    assert_eq!(cache.api_key.as_deref(), Some("pool-live-key"));

    sqlx::query(
        r#"
            UPDATE pool_routing_settings
            SET encrypted_api_key = ?1
            WHERE id = ?2
            "#,
    )
    .bind("not-a-valid-ciphertext")
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(&state.pool)
    .await
    .expect("poison encrypted api key");

    assert!(
        refresh_pool_routing_runtime_cache(state.as_ref())
            .await
            .is_err(),
        "refresh should fail once the stored api key becomes unreadable"
    );
    let cached = state.pool_routing_runtime_cache.lock().await.clone();
    assert_eq!(
        cached.as_ref().and_then(|value| value.api_key.as_deref()),
        Some("pool-live-key"),
        "failed refreshes should keep the last working routing cache in memory"
    );
}

pub(crate) fn maintenance_candidates(
    id: i64,
    status: &str,
    last_synced_at: Option<&str>,
    last_error_at: Option<&str>,
    token_expires_at: Option<&str>,
    primary_used_percent: Option<f64>,
    secondary_used_percent: Option<f64>,
) -> MaintenanceCandidateRow {
    MaintenanceCandidateRow {
        id,
        status: status.to_string(),
        last_synced_at: last_synced_at.map(ToOwned::to_owned),
        last_action_source: None,
        last_action_at: None,
        last_selected_at: None,
        last_error_at: last_error_at.map(ToOwned::to_owned),
        last_error: None,
        last_route_failure_at: None,
        last_route_failure_kind: None,
        last_action_reason_code: None,
        cooldown_until: None,
        temporary_route_failure_streak_started_at: None,
        token_expires_at: token_expires_at.map(ToOwned::to_owned),
        primary_used_percent,
        primary_resets_at: None,
        secondary_used_percent,
        secondary_resets_at: None,
        credits_has_credits: None,
        credits_unlimited: None,
        credits_balance: None,
    }
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_prioritizes_forced_accounts_and_overflow() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let refresh_lead_time = Duration::from_secs(15 * 60);

    let mut recent_error = maintenance_candidates(
        3,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        None,
        Some("2026-03-23T11:58:30Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(8.0),
        Some(8.0),
    );
    recent_error.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    recent_error.last_action_at = Some("2026-03-23T11:58:30Z".to_string());

    let mut stale_error = maintenance_candidates(
        4,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        None,
        Some("2026-03-23T11:50:00Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(9.0),
        Some(9.0),
    );
    stale_error.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    stale_error.last_action_at = Some("2026-03-23T11:50:00Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![
            maintenance_candidates(
                1,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                Some("2026-03-23T11:40:00Z"),
                None,
                Some("2026-04-23T12:00:00Z"),
                Some(15.0),
                Some(10.0),
            ),
            maintenance_candidates(
                2,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                Some("2026-03-23T11:20:00Z"),
                None,
                Some("2026-04-23T12:00:00Z"),
                Some(12.0),
                Some(22.0),
            ),
            recent_error,
            stale_error,
            maintenance_candidates(
                5,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                Some("2026-03-23T11:50:00Z"),
                None,
                Some("2026-04-23T12:00:00Z"),
                Some(5.0),
                None,
            ),
            maintenance_candidates(
                6,
                UPSTREAM_ACCOUNT_STATUS_ACTIVE,
                Some("2026-03-23T11:54:00Z"),
                None,
                Some("2026-03-23T12:10:00Z"),
                Some(4.0),
                Some(4.0),
            ),
        ],
        settings,
        refresh_lead_time,
        now,
    );

    let plan_map = plans
        .into_iter()
        .map(|plan| (plan.account_id, (plan.tier, plan.sync_interval_secs)))
        .collect::<HashMap<_, _>>();
    assert_eq!(plan_map.get(&1), Some(&(MaintenanceTier::Priority, 300)));
    assert_eq!(plan_map.get(&2), Some(&(MaintenanceTier::Secondary, 1800)));
    assert_eq!(plan_map.get(&4), Some(&(MaintenanceTier::Priority, 300)));
    assert_eq!(plan_map.get(&5), Some(&(MaintenanceTier::Priority, 300)));
    assert_eq!(plan_map.get(&6), Some(&(MaintenanceTier::Priority, 300)));
    assert!(!plan_map.contains_key(&3));
}

#[test]
pub(crate) fn maintenance_plan_is_not_due_during_upstream_rejected_cooldown() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());

    assert!(
        !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "active upstream-rejected cooldown should suppress maintenance scheduling"
    );
}

#[test]
pub(crate) fn maintenance_plan_prefers_explicit_upstream_rejected_cooldown_until() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.cooldown_until = Some("2026-04-13T17:00:00Z".to_string());

    assert!(
        !maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "explicit cooldown_until should be the canonical maintenance suppression signal"
    );
}

#[test]
pub(crate) fn maintenance_plan_is_due_when_reset_window_passes_even_during_upstream_rejected_cooldown()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-04-13T11:59:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.primary_resets_at = Some(format_utc_iso(now - ChronoDuration::minutes(1)));
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "reset-due accounts should bypass the temporary upstream-rejected cooldown"
    );
}

#[test]
pub(crate) fn maintenance_plan_ignores_wrapped_upstream_auth_errors_for_cooldown_blocking() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        42,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_error = Some(
        "oauth_upstream_rejected_request: pool upstream responded with 403: Forbidden".to_string(),
    );
    candidate.last_action_reason_code = Some("upstream_http_403".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH.to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "wrapped upstream auth errors should not enter the maintenance cooldown path"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_does_not_let_cooldown_blocked_accounts_consume_priority_slots()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };

    let mut cooldown_blocked = maintenance_candidates(
        1,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:58:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(5.0),
        Some(5.0),
    );
    cooldown_blocked.last_action_reason_code = Some("upstream_http_402".to_string());
    cooldown_blocked.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    cooldown_blocked.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    cooldown_blocked.last_action_at = Some("2026-04-13T11:58:00Z".to_string());

    let healthy_due = maintenance_candidates(
        2,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![cooldown_blocked, healthy_due],
        settings,
        Duration::from_secs(900),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].account_id, 2);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(
        plans[0].sync_interval_secs,
        settings.primary_sync_interval_secs
    );
}

#[test]
pub(crate) fn maintenance_plan_is_due_for_call_driven_upstream_rejected_errors() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        77,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:00:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.last_action_reason_code = Some("upstream_http_402".to_string());
    candidate.last_route_failure_at = Some("2026-04-13T11:00:00Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_402.to_string());
    candidate.last_error = Some("deactivated_workspace".to_string());

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "ordinary routed 402s should still allow maintenance to retry promptly"
    );
}

#[test]
pub(crate) fn maintenance_plan_is_due_for_generic_403_upstream_rejected_text() {
    let now = Utc
        .with_ymd_and_hms(2026, 4, 13, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        88,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-04-13T05:00:00Z"),
        Some("2026-04-13T11:55:00Z"),
        Some("2026-05-13T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-04-13T11:55:00Z".to_string());
    candidate.last_action_reason_code = Some("upstream_http_403".to_string());
    candidate.last_route_failure_at = Some("2026-04-13T11:55:00Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH.to_string());
    candidate.last_error = Some(
        "usage endpoint returned 403 Forbidden: upstream rejected request by policy".to_string(),
    );

    assert!(
        maintenance_plan_is_due(&candidate, MaintenanceTier::Priority, settings, now),
        "generic 403 text that happens to mention upstream rejected should not trigger the 402-only maintenance cooldown"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_keeps_refresh_due_accounts_on_primary_cadence()
{
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![maintenance_candidates(
            7,
            UPSTREAM_ACCOUNT_STATUS_ACTIVE,
            Some("2026-03-23T11:59:00Z"),
            None,
            Some("2026-03-23T12:10:00Z"),
            Some(6.0),
            Some(6.0),
        )],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert!(
        plans.is_empty(),
        "refresh-due accounts should stay on the configured primary cadence until the interval elapses"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_routes_working_accounts_to_high_frequency() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        8,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_keeps_working_refresh_due_accounts_high_frequency()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        81,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-03-23T12:10:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_routes_degraded_accounts_to_high_frequency() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        9,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        Some("2026-03-23T11:59:45Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_route_failure_at = Some("2026-03-23T11:59:45Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE.to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::HighFrequency);
    assert_eq!(plans[0].sync_interval_secs, 60);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_keeps_credits_exhausted_accounts_out_of_high_frequency()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 1,
    };
    let mut candidate = maintenance_candidates(
        91,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_selected_at = Some("2026-03-23T11:59:30Z".to_string());
    candidate.credits_has_credits = Some(1);
    candidate.credits_unlimited = Some(0);
    candidate.credits_balance = Some("0".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 0);
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_triggers_reset_due_sync_before_interval() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };
    let mut candidate = maintenance_candidates(
        10,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(plans[0].sync_interval_secs, 300);
}

#[test]
pub(crate) fn maintenance_reset_due_only_triggers_once_per_reset_boundary() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut before_reset_sync = maintenance_candidates(
        11,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    before_reset_sync.secondary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    assert!(maintenance_reset_due(&before_reset_sync, now));

    let mut after_reset_sync = before_reset_sync.clone();
    after_reset_sync.last_synced_at = Some("2026-03-23T11:59:30Z".to_string());
    assert!(!maintenance_reset_due(&after_reset_sync, now));
}

#[test]
pub(crate) fn maintenance_reset_due_stops_after_post_reset_failure_even_if_status_stays_active() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        12,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    assert!(maintenance_reset_due(&candidate, now));

    candidate.last_synced_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_error_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_route_failure_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_route_failure_kind = Some(PROXY_FAILURE_FAILED_CONTACT_UPSTREAM.to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE.to_string());

    assert!(!maintenance_reset_due(&candidate, now));
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_preserves_primary_cadence_after_call_driven_error()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };
    let mut candidate = maintenance_candidates(
        13,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-03-23T11:50:00Z"),
        Some("2026-03-23T11:58:30Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-03-23T11:58:30Z".to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert!(
        plans.is_empty(),
        "call-driven error transitions should still honor the configured primary cadence"
    );
}

#[test]
pub(crate) fn maintenance_reset_due_ignores_call_driven_error_after_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        14,
        UPSTREAM_ACCOUNT_STATUS_ERROR,
        Some("2026-03-23T11:58:30Z"),
        Some("2026-03-23T11:59:20Z"),
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source = Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());

    assert!(
        maintenance_reset_due(&candidate, now),
        "call-driven failures should not consume the post-reset catch-up sync"
    );
}

#[test]
pub(crate) fn maintenance_reset_due_ignores_deferred_egress_throttle_after_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        15,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    assert!(
        maintenance_reset_due(&candidate, now),
        "egress-deferred maintenance should not consume the post-reset catch-up sync"
    );
}

#[test]
pub(crate) fn maintenance_interval_is_due_respects_deferred_egress_throttle_anchor() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let mut candidate = maintenance_candidates(
        16,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:00:00Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    assert!(
        !maintenance_interval_is_due(&candidate, 300, now),
        "ordinary maintenance should still use egress-deferred actions as the retry anchor"
    );
}

#[test]
pub(crate) fn resolve_due_maintenance_dispatch_plans_requeues_reset_due_after_deferred_egress_throttle()
 {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
        .single()
        .expect("valid time");
    let settings = PoolRoutingMaintenanceSettings {
        primary_sync_interval_secs: 300,
        secondary_sync_interval_secs: 1800,
        priority_available_account_cap: 100,
    };
    let mut candidate = maintenance_candidates(
        17,
        UPSTREAM_ACCOUNT_STATUS_ACTIVE,
        Some("2026-03-23T11:58:30Z"),
        None,
        Some("2026-04-23T12:00:00Z"),
        Some(10.0),
        Some(10.0),
    );
    candidate.primary_resets_at = Some("2026-03-23T11:59:00Z".to_string());
    candidate.last_action_source =
        Some(UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE.to_string());
    candidate.last_action_at = Some("2026-03-23T11:59:20Z".to_string());
    candidate.last_action_reason_code =
        Some(UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED.to_string());

    let plans = resolve_due_maintenance_dispatch_plans(
        vec![candidate],
        settings,
        Duration::from_secs(15 * 60),
        now,
    );

    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].tier, MaintenanceTier::Priority);
    assert_eq!(plans[0].sync_interval_secs, 300);
}

pub(crate) fn test_routing_candidate(id: i64) -> AccountRoutingCandidateRow {
    AccountRoutingCandidateRow {
        id,
        plan_type: None,
        secondary_used_percent: None,
        secondary_window_minutes: None,
        secondary_resets_at: None,
        primary_used_percent: None,
        primary_window_minutes: None,
        primary_resets_at: None,
        local_primary_limit: None,
        local_secondary_limit: None,
        credits_has_credits: None,
        credits_unlimited: None,
        credits_balance: None,
        last_selected_at: Some("2026-03-23T11:00:00Z".to_string()),
        active_sticky_conversations: 0,
        in_flight_reservations: 0,
    }
}

#[test]
pub(crate) fn compare_routing_candidates_prefers_short_window_that_is_about_to_reset() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
        .single()
        .expect("valid now");
    let mut short_reset_soon = test_routing_candidate(1);
    short_reset_soon.plan_type = Some("team".to_string());
    short_reset_soon.primary_used_percent = Some(70.0);
    short_reset_soon.primary_window_minutes = Some(300);
    short_reset_soon.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(5)));
    short_reset_soon.secondary_used_percent = Some(40.0);
    short_reset_soon.secondary_window_minutes = Some(7 * 24 * 60);
    short_reset_soon.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(1)));

    let mut long_only = test_routing_candidate(2);
    long_only.plan_type = Some("free".to_string());
    long_only.secondary_used_percent = Some(30.0);
    long_only.secondary_window_minutes = Some(7 * 24 * 60);
    long_only.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

    assert_eq!(
        compare_routing_candidates_at(&short_reset_soon, &long_only, now),
        std::cmp::Ordering::Less,
        "a short-window account that is about to reset should beat a lower-used long-only account whose reset is still far away",
    );
}

#[test]
pub(crate) fn compare_routing_candidates_penalizes_far_from_reset_pressure() {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 26, 7, 0, 0)
        .single()
        .expect("valid now");
    let mut stretched_team = test_routing_candidate(1);
    stretched_team.plan_type = Some("team".to_string());
    stretched_team.primary_used_percent = Some(95.0);
    stretched_team.primary_window_minutes = Some(300);
    stretched_team.primary_resets_at = Some(format_utc_iso(now + ChronoDuration::minutes(250)));
    stretched_team.secondary_used_percent = Some(80.0);
    stretched_team.secondary_window_minutes = Some(7 * 24 * 60);
    stretched_team.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(6)));

    let mut healthier_long = test_routing_candidate(2);
    healthier_long.plan_type = Some("free".to_string());
    healthier_long.secondary_used_percent = Some(20.0);
    healthier_long.secondary_window_minutes = Some(7 * 24 * 60);
    healthier_long.secondary_resets_at = Some(format_utc_iso(now + ChronoDuration::days(3)));

    assert_eq!(
        compare_routing_candidates_at(&stretched_team, &healthier_long, now),
        std::cmp::Ordering::Greater,
        "near-exhausted windows with lots of time left should sort behind healthier long-window accounts",
    );
}
