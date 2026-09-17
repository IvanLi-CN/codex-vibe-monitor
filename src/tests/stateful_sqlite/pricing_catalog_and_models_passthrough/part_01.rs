use super::*;
use serde_json::json;
fn run_pricing_future_with_large_stack<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    std::thread::Builder::new()
        .name("pricing-models-large-stack".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack test runtime")
                .block_on(future)
        })
        .expect("spawn large-stack test worker")
        .join()
        .expect("join large-stack test worker")
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
            'gpt-5.6-luna'
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
}

#[tokio::test]
async fn new_sqlite_default_pricing_catalog_uses_latest_gpt_5_6_terra_and_luna_rates() {
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

    assert_custom_catalog_preserves_luna_pricing(&pool).await;
}

async fn assert_custom_catalog_preserves_luna_pricing(pool: &SqlitePool) {
    sqlx::query("UPDATE pricing_settings_meta SET catalog_version = 'custom-ci' WHERE id = ?1")
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .execute(pool)
        .await
        .expect("set custom pricing catalog version for test");
    sqlx::query(
        "UPDATE pricing_settings_models SET input_per_1m = 1.0, output_per_1m = 6.0, cache_input_per_1m = 0.10, cache_read_per_1m = 0.10, cache_write_per_1m = 1.25, source = 'official' WHERE model = 'gpt-5.6-luna'",
    )
    .execute(pool)
    .await
    .expect("restore old luna pricing in custom catalog for test");
    let catalog = load_pricing_catalog(pool)
        .await
        .expect("load custom pricing catalog should not refresh rows");
    assert_eq!(catalog.version, "custom-ci");
    let luna = catalog
        .models
        .get("gpt-5.6-luna")
        .expect("luna pricing exists");
    assert_eq!(luna.input_per_1m, 1.0);
    assert_eq!(luna.cache_read_per_1m, Some(0.10));
    assert_eq!(luna.cache_write_per_1m, Some(1.25));
    assert_eq!(luna.output_per_1m, 6.0);
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
            'gpt-5.6-luna'
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

    assert_pricing_rows_are_not_overridden(&pool).await;
}

async fn assert_pricing_rows_are_not_overridden(pool: &SqlitePool) {
    let catalog = load_pricing_catalog(pool)
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

async fn seed_pool_models_route(state: &Arc<AppState>) -> HeaderMap {
    seed_pool_routing_api_key(state, "pool-live-key").await;
    insert_test_pool_api_key_account(state, "Primary", "upstream-primary").await;
    HeaderMap::from_iter([(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer pool-live-key"),
    )])
}

#[test]
fn proxy_openai_v1_models_passthrough_when_hijack_disabled() {
    run_pricing_future_with_large_stack(async move {
        let (upstream_base, upstream_handle) = spawn_test_upstream().await;
        let state = test_state_with_openai_base(
            Url::parse(&upstream_base).expect("valid upstream base url"),
        )
        .await;
        reset_proxy_capture_hot_path_raw_fallbacks();
        let headers = seed_pool_models_route(&state).await;

        let response = proxy_openai_v1(
            State(state),
            OriginalUri("/v1/models".parse().expect("valid uri")),
            Method::GET,
            headers,
            Body::empty(),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
        let ids = extract_model_ids(&payload);
        assert_eq!(
            ids,
            vec!["upstream-model-a".to_string(), "gpt-5.2-codex".to_string()]
        );

        upstream_handle.abort();
    });
}
