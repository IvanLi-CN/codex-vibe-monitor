use super::*;
use crate::api::{
    ManagedModelDeleteRequest, ModelsDevSyncApplyRequest, delete_managed_model,
    post_models_sync_apply,
};

#[test]
fn normalize_enabled_preset_models_keeps_static_order_and_dynamic_models() {
    assert_eq!(
        normalize_enabled_preset_models(vec![
            "custom-model".to_string(),
            "gpt-6-sol".to_string(),
            "gpt-5.2-codex".to_string(),
            "custom-model".to_string(),
        ]),
        vec![
            "gpt-6-sol".to_string(),
            "gpt-5.2-codex".to_string(),
            "custom-model".to_string(),
        ]
    );
}

#[tokio::test]
async fn pricing_settings_api_keeps_empty_catalog_after_reload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-empty".to_string(),
            entries: vec![],
        }),
    )
    .await
    .expect("put pricing settings should allow empty catalog");

    assert_eq!(updated.catalog_version, "custom-empty");
    assert!(updated.entries.is_empty());

    let first_reload = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should load after update");
    assert_eq!(first_reload.version, "custom-empty");
    assert!(first_reload.models.is_empty());

    let second_reload = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should stay empty across reloads");
    assert_eq!(second_reload.version, "custom-empty");
    assert!(second_reload.models.is_empty());
}

#[tokio::test]
async fn pricing_settings_api_rejects_invalid_payload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let err = put_pricing_settings(
        State(state),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "   ".to_string(),
            entries: vec![],
        }),
    )
    .await
    .expect_err("blank catalog version should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pricing_settings_api_mirrors_legacy_cache_input_into_cache_read_response() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-legacy-cache".to_string(),
            entries: vec![PricingEntry {
                model: "gpt-legacy".to_string(),
                input_per_1m: 9.0,
                output_per_1m: 19.0,
                cache_input_per_1m: Some(0.9),
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            }],
        }),
    )
    .await
    .expect("put pricing settings should succeed");

    assert_eq!(updated.entries.len(), 1);
    assert_eq!(updated.entries[0].cache_input_per_1m, Some(0.9));
    assert_eq!(updated.entries[0].cache_read_per_1m, Some(0.9));
    assert_eq!(updated.entries[0].cache_write_per_1m, None);

    let persisted = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should load");
    let model = persisted
        .models
        .get("gpt-legacy")
        .expect("legacy model should persist");
    assert_eq!(model.cache_input_per_1m, Some(0.9));
    assert_eq!(model.cache_read_per_1m, Some(0.9));
    assert_eq!(model.cache_write_per_1m, None);
}

#[tokio::test]
async fn pricing_settings_api_reload_prefers_explicit_cache_read_over_legacy_alias() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET cache_input_per_1m = 0.99,
            cache_read_per_1m = 0.44
        WHERE model = 'gpt-5.4'
        "#,
    )
    .execute(&pool)
    .await
    .expect("prepare conflicting cache pricing row");

    let persisted = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should load");
    let model = persisted
        .models
        .get("gpt-5.4")
        .expect("gpt-5.4 should exist");
    assert_eq!(model.cache_input_per_1m, Some(0.44));
    assert_eq!(model.cache_read_per_1m, Some(0.44));
}

#[tokio::test]
async fn ensure_schema_backfills_cache_read_from_legacy_cache_input_column() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET cache_input_per_1m = 0.42,
            cache_read_per_1m = NULL
        WHERE model = 'gpt-5.4'
        "#,
    )
    .execute(&pool)
    .await
    .expect("prepare legacy cache pricing row");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let cache_read = sqlx::query_scalar::<_, Option<f64>>(
        r#"
        SELECT cache_read_per_1m
        FROM pricing_settings_models
        WHERE model = 'gpt-5.4'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load cache_read_per_1m");
    assert_eq!(cache_read, Some(0.42));
}

#[tokio::test]
async fn seed_default_pricing_catalog_prefers_explicit_cache_read_from_legacy_file() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-conflict-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-custom-v2",
  "models": {
"gpt-legacy": {
  "input_per_1m": 9.9,
  "output_per_1m": 19.9,
  "cache_input_per_1m": 0.99,
  "cache_read_per_1m": 0.44,
  "reasoning_per_1m": null
}
  }
}"#,
    )
    .expect("write conflicting legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog from legacy file");

    let _ = fs::remove_file(&legacy_path);

    let migrated = load_pricing_catalog(&pool)
        .await
        .expect("load migrated pricing catalog");
    let model = migrated
        .models
        .get("gpt-legacy")
        .expect("legacy model should be migrated");
    assert_eq!(model.cache_input_per_1m, Some(0.44));
    assert_eq!(model.cache_read_per_1m, Some(0.44));
}

#[tokio::test]
async fn seed_default_pricing_catalog_migrates_legacy_file_when_present() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-custom-v1",
  "models": {
"gpt-legacy": {
  "input_per_1m": 9.9,
  "output_per_1m": 19.9,
  "cache_input_per_1m": 0.99,
  "reasoning_per_1m": null
}
  }
}"#,
    )
    .expect("write legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog from legacy file");

    let _ = fs::remove_file(&legacy_path);

    let migrated = load_pricing_catalog(&pool)
        .await
        .expect("load migrated pricing catalog");
    assert_eq!(migrated.version, "legacy-custom-v1");
    assert_eq!(migrated.models.len(), 1);
    let model = migrated
        .models
        .get("gpt-legacy")
        .expect("legacy model should be migrated");
    assert_eq!(model.input_per_1m, 9.9);
    assert_eq!(model.output_per_1m, 19.9);
    assert_eq!(model.cache_input_per_1m, Some(0.99));
    assert_eq!(model.cache_read_per_1m, Some(0.99));
    assert_eq!(model.cache_write_per_1m, None);
    assert_eq!(model.source, "custom");
}

#[tokio::test]
async fn seed_default_pricing_catalog_falls_back_when_legacy_file_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-empty-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-empty",
  "models": {}
}"#,
    )
    .expect("write empty legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog should fall back to defaults");

    let _ = fs::remove_file(&legacy_path);

    let seeded = load_pricing_catalog(&pool)
        .await
        .expect("load seeded pricing catalog");
    assert_eq!(seeded.version, DEFAULT_PRICING_CATALOG_VERSION);
    assert!(
        seeded.models.contains_key("gpt-5.2-codex"),
        "default pricing catalog should be seeded"
    );
}

#[tokio::test]
async fn seed_default_pricing_catalog_auto_inserts_new_models_for_previous_default_version() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("downgrade pricing catalog version for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set previous default pricing catalog version for test");
    sqlx::query(
        r#"
        DELETE FROM pricing_settings_models
        WHERE model IN (
            'gpt-5.4-mini',
            'gpt-5.5',
            'gpt-5.5-pro',
            'gpt-5.6-sol',
            'gpt-5.6-terra',
            'gpt-5.6-luna',
            'gpt-6-astra',
            'gpt-6-sol',
            'gpt-6-terra',
            'gpt-6-luna'
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("delete new pricing models for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    assert_eq!(catalog.version, DEFAULT_PRICING_CATALOG_VERSION);
    assert!(catalog.models.contains_key("gpt-5.4-mini"));
    assert!(catalog.models.contains_key("gpt-5.5"));
    assert!(catalog.models.contains_key("gpt-5.5-pro"));
    assert!(catalog.models.contains_key("gpt-5.6-sol"));
    assert!(catalog.models.contains_key("gpt-5.6-terra"));
    assert!(catalog.models.contains_key("gpt-5.6-luna"));
    assert!(catalog.models.contains_key("gpt-6-astra"));
    assert!(catalog.models.contains_key("gpt-6-sol"));
    assert!(catalog.models.contains_key("gpt-6-terra"));
    assert!(catalog.models.contains_key("gpt-6-luna"));
}

#[tokio::test]
async fn new_sqlite_default_pricing_catalog_uses_official_gpt_6_rates_and_retains_terra() {
    let pool = test_current_schema_pool().await;
    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load seeded default pricing catalog");

    assert_eq!(catalog.version, DEFAULT_PRICING_CATALOG_VERSION);

    let terra = catalog
        .models
        .get("gpt-5.6-terra")
        .expect("gpt-5.6-terra default pricing should exist");
    assert_eq!(terra.input_per_1m, 2.0);
    assert_eq!(terra.cache_input_per_1m, Some(0.20));
    assert_eq!(terra.cache_read_per_1m, Some(0.20));
    assert_eq!(terra.cache_write_per_1m, Some(2.5));
    assert_eq!(terra.output_per_1m, 12.0);

    let luna = catalog
        .models
        .get("gpt-5.6-luna")
        .expect("gpt-5.6-luna default pricing should exist");
    assert_eq!(luna.input_per_1m, 0.20);
    assert_eq!(luna.cache_input_per_1m, Some(0.02));
    assert_eq!(luna.cache_read_per_1m, Some(0.02));
    assert_eq!(luna.cache_write_per_1m, Some(0.25));
    assert_eq!(luna.output_per_1m, 1.20);

    for (model, input, cache, write, output, source) in [
        ("gpt-6-astra", 10.0, 1.0, 12.5, 50.0, "official"),
        ("gpt-6-sol", 2.0, 0.2, 2.5, 10.0, "official"),
        ("gpt-6-terra", 2.0, 0.20, 2.5, 12.0, "temporary"),
        ("gpt-6-luna", 0.1, 0.01, 0.125, 0.5, "official"),
    ] {
        let pricing = catalog
            .models
            .get(model)
            .unwrap_or_else(|| panic!("{model} default pricing should exist"));
        assert_eq!(pricing.input_per_1m, input);
        assert_eq!(pricing.cache_input_per_1m, Some(cache));
        assert_eq!(pricing.cache_read_per_1m, Some(cache));
        assert_eq!(pricing.cache_write_per_1m, Some(write));
        assert_eq!(pricing.output_per_1m, output);
        assert_eq!(pricing.source, source);
    }
}

#[tokio::test]
async fn seed_default_pricing_catalog_migrates_only_unchanged_temporary_gpt_6_seeds() {
    let pool = test_current_schema_pool().await;

    sqlx::query("UPDATE pricing_settings_meta SET catalog_version = ?1 WHERE id = ?2")
        .bind(PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION)
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("set prior repo-managed catalog version");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 5.0,
            output_per_1m = 30.0,
            cache_input_per_1m = 0.5,
            cache_read_per_1m = 0.5,
            cache_write_per_1m = 6.25,
            source = 'temporary'
        WHERE model = 'gpt-6-sol'
        "#,
    )
    .execute(&pool)
    .await
    .expect("restore prior Sol seed");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 0.2,
            output_per_1m = 1.2,
            cache_input_per_1m = 0.02,
            cache_read_per_1m = 0.02,
            cache_write_per_1m = 0.25,
            source = 'temporary'
        WHERE model = 'gpt-6-luna'
        "#,
    )
    .execute(&pool)
    .await
    .expect("restore prior Luna seed");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 7.0,
            output_per_1m = 42.0,
            cache_input_per_1m = 0.7,
            cache_read_per_1m = 0.7,
            cache_write_per_1m = 8.75
        WHERE model = 'gpt-6-terra'
        "#,
    )
    .execute(&pool)
    .await
    .expect("customize retained Terra row");
    sqlx::query(
        "INSERT INTO codex_invocations (id, invoke_id, occurred_at, source, cost, raw_response) VALUES (881001, 'gpt6-price-migration-history', '2026-09-01 00:00:00', 'proxy', 0.123, '{}')",
    )
    .execute(&pool)
    .await
    .expect("insert historical non-null cost");

    for _ in 0..2 {
        seed_default_pricing_catalog(&pool)
            .await
            .expect("migrate repo-managed catalog");
    }
    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load migrated catalog");

    for (model, input, cache, write, output) in [
        ("gpt-6-astra", 10.0, 1.0, 12.5, 50.0),
        ("gpt-6-sol", 2.0, 0.2, 2.5, 10.0),
        ("gpt-6-luna", 0.1, 0.01, 0.125, 0.5),
    ] {
        let pricing = catalog
            .models
            .get(model)
            .expect("official GPT-6 row exists");
        assert_eq!(pricing.input_per_1m, input);
        assert_eq!(pricing.cache_read_per_1m, Some(cache));
        assert_eq!(pricing.cache_write_per_1m, Some(write));
        assert_eq!(pricing.output_per_1m, output);
        assert_eq!(pricing.source, "official");
    }
    let terra = catalog.models.get("gpt-6-terra").expect("Terra row exists");
    assert_eq!(terra.input_per_1m, 7.0);
    assert_eq!(terra.output_per_1m, 42.0);
    assert_eq!(terra.source, "temporary");
    let historical_cost =
        sqlx::query_scalar::<_, f64>("SELECT cost FROM codex_invocations WHERE id = 881001")
            .fetch_one(&pool)
            .await
            .expect("load preserved historical cost");
    assert_eq!(historical_cost, 0.123);
}

#[tokio::test]
async fn seed_default_pricing_catalog_preserves_edited_or_custom_gpt_6_seed_rows() {
    let pool = test_current_schema_pool().await;
    sqlx::query("UPDATE pricing_settings_meta SET catalog_version = ?1 WHERE id = ?2")
        .bind(PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION)
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(&pool)
        .await
        .expect("set prior repo-managed catalog version");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 5.0,
            output_per_1m = 30.0,
            cache_input_per_1m = 0.5,
            cache_read_per_1m = 0.5,
            cache_write_per_1m = 6.25,
            source = 'custom'
        WHERE model = 'gpt-6-sol'
        "#,
    )
    .execute(&pool)
    .await
    .expect("mark Sol pricing custom");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 0.21,
            output_per_1m = 1.2,
            cache_input_per_1m = 0.02,
            cache_read_per_1m = 0.02,
            cache_write_per_1m = 0.25,
            source = 'temporary'
        WHERE model = 'gpt-6-luna'
        "#,
    )
    .execute(&pool)
    .await
    .expect("edit Luna seed pricing");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load catalog without replacing customized GPT-6 rows");
    let sol = catalog.models.get("gpt-6-sol").expect("Sol row exists");
    assert_eq!(sol.input_per_1m, 5.0);
    assert_eq!(sol.source, "custom");
    let luna = catalog.models.get("gpt-6-luna").expect("Luna row exists");
    assert_eq!(luna.input_per_1m, 0.21);
    assert_eq!(luna.source, "temporary");
}

#[tokio::test]
async fn seed_default_pricing_catalog_refreshes_unchanged_official_gpt_5_6_terra_and_luna_rows() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set previous pricing catalog version for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 2.5,
            output_per_1m = 15.0,
            cache_input_per_1m = 0.25,
            cache_read_per_1m = NULL,
            cache_write_per_1m = 3.125,
            source = 'official'
        WHERE model = 'gpt-5.6-terra'
        "#,
    )
    .execute(&pool)
    .await
    .expect("restore old terra official pricing for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 1.0,
            output_per_1m = 6.0,
            cache_input_per_1m = 0.10,
            cache_read_per_1m = 0.10,
            cache_write_per_1m = 1.25,
            source = 'official'
        WHERE model = 'gpt-5.6-luna'
        "#,
    )
    .execute(&pool)
    .await
    .expect("restore old luna official pricing for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should refresh unchanged defaults");
    assert_eq!(catalog.version, DEFAULT_PRICING_CATALOG_VERSION);
    let terra = catalog
        .models
        .get("gpt-5.6-terra")
        .expect("gpt-5.6-terra pricing should exist");
    assert_eq!(terra.input_per_1m, 2.0);
    assert_eq!(terra.cache_read_per_1m, Some(0.20));
    assert_eq!(terra.cache_write_per_1m, Some(2.5));
    assert_eq!(terra.output_per_1m, 12.0);
    let luna = catalog
        .models
        .get("gpt-5.6-luna")
        .expect("gpt-5.6-luna pricing should exist");
    assert_eq!(luna.input_per_1m, 0.20);
    assert_eq!(luna.cache_read_per_1m, Some(0.02));
    assert_eq!(luna.cache_write_per_1m, Some(0.25));
    assert_eq!(luna.output_per_1m, 1.20);
}

#[tokio::test]
async fn seed_default_pricing_catalog_preserves_changed_or_custom_gpt_5_6_rows() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set previous pricing catalog version for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 2.6,
            output_per_1m = 15.0,
            cache_input_per_1m = 0.25,
            cache_read_per_1m = 0.25,
            cache_write_per_1m = 3.125,
            source = 'official'
        WHERE model = 'gpt-5.6-terra'
        "#,
    )
    .execute(&pool)
    .await
    .expect("customize terra pricing for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 1.0,
            output_per_1m = 6.0,
            cache_input_per_1m = 0.10,
            cache_read_per_1m = 0.10,
            cache_write_per_1m = 1.25,
            source = 'custom'
        WHERE model = 'gpt-5.6-luna'
        "#,
    )
    .execute(&pool)
    .await
    .expect("mark luna pricing custom for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should preserve custom rows");
    assert_eq!(catalog.version, DEFAULT_PRICING_CATALOG_VERSION);
    let terra = catalog
        .models
        .get("gpt-5.6-terra")
        .expect("gpt-5.6-terra pricing should exist");
    assert_eq!(terra.input_per_1m, 2.6);
    assert_eq!(terra.cache_read_per_1m, Some(0.25));
    assert_eq!(terra.cache_write_per_1m, Some(3.125));
    assert_eq!(terra.output_per_1m, 15.0);
    let luna = catalog
        .models
        .get("gpt-5.6-luna")
        .expect("gpt-5.6-luna pricing should exist");
    assert_eq!(luna.input_per_1m, 1.0);
    assert_eq!(luna.cache_read_per_1m, Some(0.10));
    assert_eq!(luna.cache_write_per_1m, Some(1.25));
    assert_eq!(luna.output_per_1m, 6.0);
    assert_eq!(luna.source, "custom");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = 'custom-ci'
        WHERE id = ?1
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set custom pricing catalog version for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = 1.0,
            output_per_1m = 6.0,
            cache_input_per_1m = 0.10,
            cache_read_per_1m = 0.10,
            cache_write_per_1m = 1.25,
            source = 'official'
        WHERE model = 'gpt-5.6-luna'
        "#,
    )
    .execute(&pool)
    .await
    .expect("restore old luna pricing in custom catalog for test");

    let custom_catalog = load_pricing_catalog(&pool)
        .await
        .expect("load custom pricing catalog should not refresh rows");
    assert_eq!(custom_catalog.version, "custom-ci");
    let custom_luna = custom_catalog
        .models
        .get("gpt-5.6-luna")
        .expect("gpt-5.6-luna pricing should exist");
    assert_eq!(custom_luna.input_per_1m, 1.0);
    assert_eq!(custom_luna.cache_read_per_1m, Some(0.10));
    assert_eq!(custom_luna.cache_write_per_1m, Some(1.25));
    assert_eq!(custom_luna.output_per_1m, 6.0);
}

#[tokio::test]
async fn seed_default_pricing_catalog_normalizes_gpt_5_3_codex_source_for_legacy_default_version() {
    let pool = test_current_schema_pool().await;

    save_pricing_catalog(&pool, &default_pricing_catalog())
        .await
        .expect("seed default pricing catalog");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("downgrade catalog version for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET source = 'temporary'
        WHERE model = 'gpt-5.3-codex'
        "#,
    )
    .execute(&pool)
    .await
    .expect("force legacy gpt-5.3-codex source");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog");
    let pricing = catalog
        .models
        .get("gpt-5.3-codex")
        .expect("gpt-5.3-codex pricing present");
    assert_eq!(pricing.source, "official");
}

#[tokio::test]
async fn seed_default_pricing_catalog_does_not_auto_insert_new_models_for_custom_catalog_version() {
    let pool = test_current_schema_pool().await;

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind("custom-ci")
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set custom pricing catalog version for test");
    sqlx::query(
        r#"
        DELETE FROM pricing_settings_models
        WHERE model IN (
            'gpt-5.4',
            'gpt-5.4-pro',
            'gpt-5.4-mini',
            'gpt-5.5',
            'gpt-5.5-pro',
            'gpt-5.6-sol',
            'gpt-5.6-terra',
            'gpt-5.6-luna',
            'gpt-6-sol',
            'gpt-6-terra',
            'gpt-6-luna'
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("delete new pricing models for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    assert!(!catalog.models.contains_key("gpt-5.4"));
    assert!(!catalog.models.contains_key("gpt-5.4-pro"));
    assert!(!catalog.models.contains_key("gpt-5.4-mini"));
    assert!(!catalog.models.contains_key("gpt-5.5"));
    assert!(!catalog.models.contains_key("gpt-5.5-pro"));
    assert!(!catalog.models.contains_key("gpt-5.6-sol"));
    assert!(!catalog.models.contains_key("gpt-5.6-terra"));
    assert!(!catalog.models.contains_key("gpt-5.6-luna"));
    assert!(!catalog.models.contains_key("gpt-6-sol"));
    assert!(!catalog.models.contains_key("gpt-6-terra"));
    assert!(!catalog.models.contains_key("gpt-6-luna"));
}

#[tokio::test]
async fn seed_default_pricing_catalog_does_not_override_existing_pricing_for_new_models() {
    let pool = test_current_schema_pool().await;

    // Simulate a repo-managed default catalog version so startup seeding will call
    // ensure_pricing_models_present, which must not overwrite existing rows.
    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set legacy pricing catalog version for test");
    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set previous default pricing catalog version for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = ?1,
            output_per_1m = ?2,
            cache_input_per_1m = ?3,
            cache_read_per_1m = ?3,
            source = 'custom'
        WHERE model = 'gpt-5.4'
        "#,
    )
    .bind(99.0)
    .bind(199.0)
    .bind(Some(9.9))
    .execute(&pool)
    .await
    .expect("override gpt-5.4 pricing for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = ?1,
            output_per_1m = ?2,
            source = 'custom'
        WHERE model = 'gpt-5.4-pro'
        "#,
    )
    .bind(88.0)
    .bind(188.0)
    .execute(&pool)
    .await
    .expect("override gpt-5.4-pro pricing for test");
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO pricing_settings_models (
            model,
            input_per_1m,
            output_per_1m,
            cache_input_per_1m,
            cache_read_per_1m,
            cache_write_per_1m,
            reasoning_per_1m,
            source
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("gpt-5.6-sol")
    .bind(55.0)
    .bind(155.0)
    .bind(Some(5.5))
    .bind(Some(5.5))
    .bind(Some(7.5))
    .bind(Some(1.5))
    .bind("custom")
    .execute(&pool)
    .await
    .expect("override gpt-5.6-sol pricing for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    let gpt_5_4 = catalog.models.get("gpt-5.4").expect("gpt-5.4 should exist");
    assert_eq!(gpt_5_4.input_per_1m, 99.0);
    assert_eq!(gpt_5_4.output_per_1m, 199.0);
    assert_eq!(gpt_5_4.cache_input_per_1m, Some(9.9));
    assert_eq!(gpt_5_4.cache_read_per_1m, Some(9.9));
    assert_eq!(gpt_5_4.source, "custom");

    let gpt_5_4_pro = catalog
        .models
        .get("gpt-5.4-pro")
        .expect("gpt-5.4-pro should exist");
    assert_eq!(gpt_5_4_pro.input_per_1m, 88.0);
    assert_eq!(gpt_5_4_pro.output_per_1m, 188.0);
    assert_eq!(gpt_5_4_pro.cache_input_per_1m, None);
    assert_eq!(gpt_5_4_pro.source, "custom");

    let gpt_5_6_sol = catalog
        .models
        .get("gpt-5.6-sol")
        .expect("gpt-5.6-sol should exist");
    assert_eq!(gpt_5_6_sol.input_per_1m, 55.0);
    assert_eq!(gpt_5_6_sol.output_per_1m, 155.0);
    assert_eq!(gpt_5_6_sol.cache_input_per_1m, Some(5.5));
    assert_eq!(gpt_5_6_sol.cache_read_per_1m, Some(5.5));
    assert_eq!(gpt_5_6_sol.cache_write_per_1m, Some(7.5));
    assert_eq!(gpt_5_6_sol.reasoning_per_1m, Some(1.5));
    assert_eq!(gpt_5_6_sol.source, "custom");
}

#[tokio::test]
async fn managed_model_catalog_migrates_preserves_deletions_and_allows_rediscovery() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;
    let pool = &state.pool;

    let mut legacy_proxy = state.proxy_model_settings.read().await.clone();
    legacy_proxy.enabled_preset_models =
        vec!["gpt-5.4".to_string(), "legacy-dynamic-model".to_string()];
    save_proxy_model_settings(pool, legacy_proxy.clone())
        .await
        .expect("save legacy enabled preset state");
    *state.proxy_model_settings.write().await = legacy_proxy.clone();

    sqlx::query("DELETE FROM managed_models")
        .execute(pool)
        .await
        .expect("remove catalog to simulate the previous schema");
    sqlx::query("DELETE FROM managed_model_catalog_migrations")
        .execute(pool)
        .await
        .expect("remove migration marker to simulate the previous schema");
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO pricing_settings_models (
            model, input_per_1m, output_per_1m, cache_input_per_1m,
            cache_read_per_1m, cache_write_per_1m, reasoning_per_1m, source
        ) VALUES ('legacy-priced-model', 1.0, 2.0, NULL, NULL, NULL, NULL, 'custom')
        "#,
    )
    .execute(pool)
    .await
    .expect("seed legacy priced model");

    ensure_managed_model_catalog(pool)
        .await
        .expect("migrate legacy model candidates");
    let migrated_models = load_managed_model_ids(pool)
        .await
        .expect("load migrated model IDs");
    assert!(migrated_models.iter().any(|model| model == "gpt-5.4"));
    assert!(
        migrated_models
            .iter()
            .any(|model| model == "legacy-priced-model")
    );
    assert!(
        migrated_models
            .iter()
            .any(|model| model == "legacy-dynamic-model")
    );
    assert_eq!(
        load_proxy_model_settings(pool)
            .await
            .expect("load migrated proxy settings")
            .enabled_preset_models,
        legacy_proxy.enabled_preset_models
    );

    let Json(updated_proxy) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: legacy_proxy.hijack_enabled,
            merge_upstream_enabled: legacy_proxy.merge_upstream_enabled,
            fast_mode_rewrite_mode: None,
            upstream_429_max_retries: None,
            websocket_enabled: None,
            upstream_websocket_default_enabled: None,
            request_body_logging_enabled: None,
            response_body_logging_enabled: None,
            encrypted_session_owner_routing_enabled: None,
            enabled_models: legacy_proxy.enabled_preset_models.clone(),
        }),
    )
    .await
    .expect("proxy settings update should preserve enabled legacy models");
    assert_eq!(
        updated_proxy.enabled_models,
        legacy_proxy.enabled_preset_models
    );

    sqlx::query(
        "INSERT INTO codex_invocations (invoke_id, occurred_at, source, status, cost, raw_response) VALUES ('model-delete-history', '2026-09-01 00:00:00', 'proxy', 'success', 0.123, '{}')",
    )
    .execute(pool)
    .await
    .expect("insert historical invocation cost");

    let Json(deleted) = delete_managed_model(
        State(state.clone()),
        HeaderMap::new(),
        Json(ManagedModelDeleteRequest {
            model: "gpt-5.4".to_string(),
        }),
    )
    .await
    .expect("delete managed model");
    assert_eq!(deleted.deleted_model, "gpt-5.4");
    assert!(
        !state
            .pricing_catalog
            .read()
            .await
            .models
            .contains_key("gpt-5.4")
    );

    ensure_managed_model_catalog(pool)
        .await
        .expect("repeat catalog initialization after restart");
    let pricing_after_restart = load_pricing_catalog(pool)
        .await
        .expect("load pricing after restart");
    assert!(!pricing_after_restart.models.contains_key("gpt-5.4"));
    assert!(
        !load_managed_model_ids(pool)
            .await
            .expect("load model IDs after restart")
            .iter()
            .any(|model| model == "gpt-5.4")
    );
    assert!(
        !load_proxy_model_settings(pool)
            .await
            .expect("load proxy settings after restart")
            .enabled_preset_models
            .iter()
            .any(|model| model == "gpt-5.4")
    );
    assert!(
        load_proxy_model_settings(pool)
            .await
            .expect("load remaining proxy settings after restart")
            .enabled_preset_models
            .iter()
            .any(|model| model == "legacy-dynamic-model")
    );
    let historical_cost = sqlx::query_scalar::<_, f64>(
        "SELECT cost FROM codex_invocations WHERE invoke_id = 'model-delete-history'",
    )
    .fetch_one(pool)
    .await
    .expect("read historical invocation cost");
    assert_eq!(historical_cost, 0.123);

    upsert_synced_model_prices(
        pool,
        &[PricingEntry {
            model: "gpt-5.4".to_string(),
            input_per_1m: 3.0,
            output_per_1m: 9.0,
            cache_input_per_1m: Some(0.3),
            cache_read_per_1m: Some(0.3),
            cache_write_per_1m: Some(0.4),
            reasoning_per_1m: None,
            source: "models.dev".to_string(),
        }],
    )
    .await
    .expect("rediscover model from source");
    let rediscovered_proxy = load_proxy_model_settings(pool)
        .await
        .expect("load proxy settings after rediscovery");
    assert!(
        !rediscovered_proxy
            .enabled_preset_models
            .iter()
            .any(|model| model == "gpt-5.4")
    );
    assert!(
        load_managed_model_ids(pool)
            .await
            .expect("load rediscovered model IDs")
            .iter()
            .any(|model| model == "gpt-5.4")
    );
}

#[tokio::test]
async fn models_dev_apply_updates_only_selected_prices_and_marks_the_source() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT OR REPLACE INTO pricing_settings_models (
            model, input_per_1m, output_per_1m, cache_input_per_1m,
            cache_read_per_1m, cache_write_per_1m, reasoning_per_1m, source
        ) VALUES ('selected-model', 10.0, 20.0, NULL, NULL, NULL, NULL, 'custom'),
                 ('unselected-model', 30.0, 40.0, NULL, NULL, NULL, NULL, 'custom')
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed local model prices");

    let Json(applied) = post_models_sync_apply(
        State(state.clone()),
        HeaderMap::new(),
        Json(ModelsDevSyncApplyRequest {
            entries: vec![PricingEntry {
                model: "selected-model".to_string(),
                input_per_1m: 1.5,
                output_per_1m: 2.5,
                cache_input_per_1m: None,
                cache_read_per_1m: Some(0.15),
                cache_write_per_1m: Some(0.2),
                reasoning_per_1m: Some(3.5),
                source: "custom".to_string(),
            }],
        }),
    )
    .await
    .expect("apply selected models.dev prices");

    let selected = applied
        .entries
        .iter()
        .find(|entry| entry.model == "selected-model")
        .expect("selected model price returned");
    assert_eq!(selected.input_per_1m, 1.5);
    assert_eq!(selected.output_per_1m, 2.5);
    assert_eq!(selected.source, "models.dev");
    let unselected = applied
        .entries
        .iter()
        .find(|entry| entry.model == "unselected-model")
        .expect("unselected model price returned");
    assert_eq!(unselected.input_per_1m, 30.0);
    assert_eq!(unselected.output_per_1m, 40.0);
    assert_eq!(unselected.source, "custom");
    let published = state.pricing_catalog.read().await.clone();
    assert_eq!(
        published.models.get("selected-model").unwrap().input_per_1m,
        1.5
    );
    assert_eq!(
        published
            .models
            .get("unselected-model")
            .unwrap()
            .input_per_1m,
        30.0
    );
    assert!(
        !state
            .proxy_model_settings
            .read()
            .await
            .enabled_preset_models
            .iter()
            .any(|model| model == "selected-model")
    );
}

#[tokio::test]
async fn model_price_sync_and_delete_roll_back_if_catalog_snapshot_fails() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query(
        r#"
        INSERT INTO pricing_settings_models (model, input_per_1m, output_per_1m, source)
        VALUES ('corrupt-catalog-row', 'not-a-number', 2.0, 'custom')
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed an unreadable catalog row");
    sqlx::query(
        r#"
        INSERT INTO pricing_settings_models (model, input_per_1m, output_per_1m, source)
        VALUES ('delete-target', 1.0, 2.0, 'custom')
        "#,
    )
    .execute(&state.pool)
    .await
    .expect("seed delete target price");
    sqlx::query("INSERT INTO managed_models (model) VALUES ('delete-target')")
        .execute(&state.pool)
        .await
        .expect("seed delete target model");

    let sync_result = post_models_sync_apply(
        State(state.clone()),
        HeaderMap::new(),
        Json(ModelsDevSyncApplyRequest {
            entries: vec![PricingEntry {
                model: "sync-target".to_string(),
                input_per_1m: 1.5,
                output_per_1m: 2.5,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "models.dev".to_string(),
            }],
        }),
    )
    .await;
    assert!(matches!(
        sync_result,
        Err((StatusCode::INTERNAL_SERVER_ERROR, _))
    ));
    let sync_target_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pricing_settings_models WHERE model = 'sync-target')",
    )
    .fetch_one(&state.pool)
    .await
    .expect("check synced price rollback");
    assert_eq!(sync_target_exists, 0);

    let delete_result = delete_managed_model(
        State(state.clone()),
        HeaderMap::new(),
        Json(ManagedModelDeleteRequest {
            model: "delete-target".to_string(),
        }),
    )
    .await;
    assert!(matches!(
        delete_result,
        Err((StatusCode::INTERNAL_SERVER_ERROR, _))
    ));
    let delete_target_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM pricing_settings_models WHERE model = 'delete-target')",
    )
    .fetch_one(&state.pool)
    .await
    .expect("check deleted price rollback");
    assert_eq!(delete_target_exists, 1);
    let managed_delete_target_exists = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM managed_models WHERE model = 'delete-target')",
    )
    .fetch_one(&state.pool)
    .await
    .expect("check managed model rollback");
    assert_eq!(managed_delete_target_exists, 1);
    let delete_target_suppressed = sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM managed_model_suppressions WHERE model = 'delete-target')",
    )
    .fetch_one(&state.pool)
    .await
    .expect("check model suppression rollback");
    assert_eq!(delete_target_suppressed, 0);
}
