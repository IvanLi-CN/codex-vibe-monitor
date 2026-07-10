use super::*;

pub(crate) async fn load_proxy_model_settings(pool: &Pool<Sqlite>) -> Result<ProxyModelSettings> {
    let row = sqlx::query_as::<_, ProxyModelSettingsRow>(
        r#"
        SELECT
            hijack_enabled,
            merge_upstream_enabled,
            upstream_429_max_retries,
            openai_proxy_websocket_enabled,
            openai_proxy_upstream_websocket_default_enabled,
            request_body_logging_enabled,
            response_body_logging_enabled,
            encrypted_session_owner_routing_enabled,
            enabled_preset_models_json
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to load proxy_model_settings row")?;

    Ok(row
        .map(Into::into)
        .unwrap_or_else(ProxyModelSettings::default))
}

pub(crate) async fn save_proxy_model_settings(
    pool: &Pool<Sqlite>,
    settings: ProxyModelSettings,
) -> Result<()> {
    let settings = settings.normalized();
    let enabled_models_json = serde_json::to_string(&settings.enabled_preset_models)
        .context("failed to serialize enabled preset models")?;
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET hijack_enabled = ?1,
            merge_upstream_enabled = ?2,
            upstream_429_max_retries = ?3,
            openai_proxy_websocket_enabled = ?4,
            openai_proxy_upstream_websocket_default_enabled = ?5,
            request_body_logging_enabled = ?6,
            response_body_logging_enabled = ?7,
            encrypted_session_owner_routing_enabled = ?8,
            encrypted_session_owner_routing_initialized = 1,
            websocket_settings_migrated = 1,
            enabled_preset_models_json = ?9,
            updated_at = datetime('now')
        WHERE id = ?10
        "#,
    )
    .bind(settings.hijack_enabled as i64)
    .bind(settings.merge_upstream_enabled as i64)
    .bind(i64::from(settings.upstream_429_max_retries))
    .bind(settings.websocket_enabled as i64)
    .bind(settings.upstream_websocket_default_enabled as i64)
    .bind(settings.request_body_logging_enabled as i64)
    .bind(settings.response_body_logging_enabled as i64)
    .bind(settings.encrypted_session_owner_routing_enabled as i64)
    .bind(enabled_models_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to persist proxy_model_settings row")?;

    Ok(())
}

const LATEST_PROXY_PRESET_MODELS_MIGRATION_VERSION: i64 = 2;

pub(crate) async fn ensure_proxy_enabled_models_contains_new_presets(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT preset_models_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to check proxy preset models migration flag")?
    .unwrap_or(0);
    if migrated >= LATEST_PROXY_PRESET_MODELS_MIGRATION_VERSION {
        return Ok(());
    }

    let mut settings = load_proxy_model_settings(pool).await?;

    if settings.enabled_preset_models.is_empty() {
        mark_proxy_preset_models_migrated(pool).await?;
        return Ok(());
    }

    let legacy_default = normalize_enabled_preset_models(
        LEGACY_PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>(),
    );
    let previous_default = normalize_enabled_preset_models(
        PREVIOUS_PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>(),
    );
    let oldest_legacy_default = normalize_enabled_preset_models(
        OLDEST_LEGACY_PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>(),
    );
    if settings.enabled_preset_models != legacy_default
        && settings.enabled_preset_models != previous_default
        && settings.enabled_preset_models != oldest_legacy_default
    {
        // Respect user customizations: only auto-append when the enabled list matches
        // a repo-managed default preset list exactly.
        mark_proxy_preset_models_migrated(pool).await?;
        return Ok(());
    }

    let mut changed = false;
    for required in [
        "gpt-5.4",
        "gpt-5.4-pro",
        "gpt-5.5",
        "gpt-5.5-pro",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
    ] {
        if !settings
            .enabled_preset_models
            .iter()
            .any(|id| id == required)
        {
            settings.enabled_preset_models.push(required.to_string());
            changed = true;
        }
    }

    if !changed {
        mark_proxy_preset_models_migrated(pool).await?;
        return Ok(());
    }

    settings.enabled_preset_models =
        normalize_enabled_preset_models(settings.enabled_preset_models);
    save_proxy_model_settings(pool, settings).await?;
    mark_proxy_preset_models_migrated(pool).await
}

pub(crate) async fn ensure_proxy_websocket_settings_initialized(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT websocket_settings_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to check websocket settings migration flag")?
    .unwrap_or(0);
    if migrated != 0 {
        return Ok(());
    }

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET openai_proxy_websocket_enabled = ?1,
            openai_proxy_upstream_websocket_default_enabled = ?2,
            websocket_settings_migrated = 1,
            updated_at = datetime('now')
        WHERE id = ?3
        "#,
    )
    .bind(config.openai_proxy_websocket_enabled as i64)
    .bind(config.openai_proxy_upstream_websocket_default_enabled as i64)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to initialize websocket settings from deployment defaults")?;

    Ok(())
}

pub(crate) async fn ensure_proxy_encrypted_session_owner_routing_setting_initialized(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
) -> Result<()> {
    let initialized = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT encrypted_session_owner_routing_initialized
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to check encrypted owner routing initialization flag")?
    .unwrap_or(0);
    if initialized != 0 {
        return Ok(());
    }

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET encrypted_session_owner_routing_enabled = ?1,
            encrypted_session_owner_routing_initialized = 1,
            updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(config.openai_proxy_encrypted_session_owner_routing_enabled as i64)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to initialize encrypted owner routing setting from deployment default")?;

    Ok(())
}

pub(crate) async fn mark_proxy_preset_models_migrated(pool: &Pool<Sqlite>) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET preset_models_migrated = ?1,
            updated_at = datetime('now')
        WHERE id = ?2
        "#,
    )
    .bind(LATEST_PROXY_PRESET_MODELS_MIGRATION_VERSION)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to mark proxy preset models as migrated")?;
    Ok(())
}

#[derive(Debug, FromRow)]
pub(crate) struct PricingSettingsMetaRow {
    catalog_version: String,
}

#[derive(Debug, FromRow)]
pub(crate) struct PricingSettingsModelRow {
    model: String,
    input_per_1m: f64,
    output_per_1m: f64,
    cache_input_per_1m: Option<f64>,
    cache_read_per_1m: Option<f64>,
    cache_write_per_1m: Option<f64>,
    reasoning_per_1m: Option<f64>,
    source: String,
}

pub(crate) async fn ensure_pricing_model_present(
    pool: &Pool<Sqlite>,
    model: &str,
    pricing: ModelPricing,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO pricing_settings_models (
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
    .bind(model)
    .bind(pricing.input_per_1m)
    .bind(pricing.output_per_1m)
    .bind(pricing.effective_cache_read_per_1m())
    .bind(pricing.effective_cache_read_per_1m())
    .bind(pricing.cache_write_per_1m)
    .bind(pricing.reasoning_per_1m)
    .bind(pricing.source)
    .execute(pool)
    .await
    .with_context(|| format!("failed to ensure pricing model exists: {model}"))?;

    Ok(())
}

pub(crate) async fn ensure_pricing_models_present(pool: &Pool<Sqlite>) -> Result<()> {
    ensure_pricing_model_present(
        pool,
        "gpt-5.4",
        ModelPricing {
            input_per_1m: 2.5,
            output_per_1m: 15.0,
            cache_input_per_1m: Some(0.25),
            cache_read_per_1m: Some(0.25),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.4-pro",
        ModelPricing {
            input_per_1m: 30.0,
            output_per_1m: 180.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.4-mini",
        ModelPricing {
            input_per_1m: 0.75,
            output_per_1m: 4.5,
            cache_input_per_1m: Some(0.075),
            cache_read_per_1m: Some(0.075),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.5",
        ModelPricing {
            input_per_1m: 5.0,
            output_per_1m: 30.0,
            cache_input_per_1m: Some(0.5),
            cache_read_per_1m: Some(0.5),
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.5-pro",
        ModelPricing {
            input_per_1m: 30.0,
            output_per_1m: 180.0,
            cache_input_per_1m: None,
            cache_read_per_1m: None,
            cache_write_per_1m: None,
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.6-sol",
        ModelPricing {
            input_per_1m: 5.0,
            output_per_1m: 30.0,
            cache_input_per_1m: Some(0.5),
            cache_read_per_1m: Some(0.5),
            cache_write_per_1m: Some(6.25),
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.6-terra",
        ModelPricing {
            input_per_1m: 2.5,
            output_per_1m: 15.0,
            cache_input_per_1m: Some(0.25),
            cache_read_per_1m: Some(0.25),
            cache_write_per_1m: Some(3.125),
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    ensure_pricing_model_present(
        pool,
        "gpt-5.6-luna",
        ModelPricing {
            input_per_1m: 1.0,
            output_per_1m: 6.0,
            cache_input_per_1m: Some(0.10),
            cache_read_per_1m: Some(0.10),
            cache_write_per_1m: Some(1.25),
            reasoning_per_1m: None,
            source: "official".to_string(),
        },
    )
    .await?;
    Ok(())
}

pub(crate) fn is_repo_managed_default_pricing_catalog_version(version: &str) -> bool {
    matches!(
        version,
        DEFAULT_PRICING_CATALOG_VERSION
            | PREVIOUS_DEFAULT_PRICING_CATALOG_VERSION
            | LEGACY_DEFAULT_PRICING_CATALOG_VERSION
            | OLDEST_LEGACY_DEFAULT_PRICING_CATALOG_VERSION
    )
}

pub(crate) async fn promote_repo_managed_default_pricing_catalog_version(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1,
            updated_at = datetime('now')
        WHERE id = ?2
          AND catalog_version <> ?1
        "#,
    )
    .bind(DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to promote repo-managed pricing catalog version")?;
    Ok(())
}

pub(crate) async fn normalize_default_pricing_sources(pool: &Pool<Sqlite>) -> Result<()> {
    // Legacy versions used `temporary` for some built-in models; keep the pricing untouched
    // but normalize the metadata so UI and reporting remain consistent.
    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET source = 'official'
        WHERE model = 'gpt-5.3-codex'
          AND lower(trim(source)) = 'temporary'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to normalize default pricing sources")?;
    Ok(())
}

pub(crate) async fn seed_default_pricing_catalog(pool: &Pool<Sqlite>) -> Result<()> {
    let legacy_path = resolve_legacy_pricing_catalog_path();
    seed_default_pricing_catalog_with_legacy_path(pool, Some(&legacy_path)).await
}

pub(crate) async fn seed_default_pricing_catalog_with_legacy_path(
    pool: &Pool<Sqlite>,
    legacy_path: Option<&Path>,
) -> Result<()> {
    let meta_exists = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pricing_settings_meta
        WHERE id = ?1
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .fetch_one(pool)
    .await
    .context("failed to count pricing_settings_meta rows")?;
    if meta_exists > 0 {
        let version = sqlx::query_scalar::<_, String>(
            r#"
            SELECT catalog_version
            FROM pricing_settings_meta
            WHERE id = ?1
            LIMIT 1
            "#,
        )
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .fetch_one(pool)
        .await
        .context("failed to load pricing_settings_meta row")?;
        if is_repo_managed_default_pricing_catalog_version(&version) {
            ensure_pricing_models_present(pool).await?;
            normalize_default_pricing_sources(pool).await?;
            promote_repo_managed_default_pricing_catalog_version(pool).await?;
        }
        return Ok(());
    }

    let existing_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM pricing_settings_models
        "#,
    )
    .fetch_one(pool)
    .await
    .context("failed to count pricing_settings_models rows")?;

    if existing_count > 0 {
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO pricing_settings_meta (id, catalog_version)
            VALUES (?1, ?2)
            "#,
        )
        .bind(PRICING_SETTINGS_SINGLETON_ID)
        .bind(DEFAULT_PRICING_CATALOG_VERSION)
        .execute(pool)
        .await
        .context("failed to ensure default pricing_settings_meta row")?;
        ensure_pricing_models_present(pool).await?;
        normalize_default_pricing_sources(pool).await?;
        return Ok(());
    }

    if let Some(path) = legacy_path {
        match load_legacy_pricing_catalog(path) {
            Ok(Some(catalog)) => {
                info!(
                    path = %path.display(),
                    version = %catalog.version,
                    model_count = catalog.models.len(),
                    "migrating legacy pricing catalog into sqlite"
                );
                save_pricing_catalog(pool, &catalog).await?;
                if is_repo_managed_default_pricing_catalog_version(&catalog.version) {
                    ensure_pricing_models_present(pool).await?;
                    normalize_default_pricing_sources(pool).await?;
                    promote_repo_managed_default_pricing_catalog_version(pool).await?;
                }
                return Ok(());
            }
            Ok(None) => {}
            Err(err) => {
                warn!(
                    path = %path.display(),
                    ?err,
                    "failed to migrate legacy pricing catalog; falling back to defaults"
                );
            }
        }
    }

    save_pricing_catalog(pool, &default_pricing_catalog()).await?;
    ensure_pricing_models_present(pool).await?;
    normalize_default_pricing_sources(pool).await?;
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyPricingCatalogFile {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    models: HashMap<String, LegacyModelPricing>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct LegacyModelPricing {
    input_per_1m: f64,
    output_per_1m: f64,
    #[serde(default)]
    cache_input_per_1m: Option<f64>,
    #[serde(default)]
    cache_read_per_1m: Option<f64>,
    #[serde(default)]
    cache_write_per_1m: Option<f64>,
    #[serde(default)]
    reasoning_per_1m: Option<f64>,
    #[serde(default)]
    source: Option<String>,
}

pub(crate) fn resolve_legacy_pricing_catalog_path() -> PathBuf {
    env::var("PROXY_PRICING_CATALOG_PATH")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_PROXY_PRICING_CATALOG_PATH))
}

pub(crate) fn load_legacy_pricing_catalog(path: &Path) -> Result<Option<PricingCatalog>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read legacy pricing catalog: {}", path.display()))?;
    let parsed: LegacyPricingCatalogFile = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse legacy pricing catalog: {}", path.display()))?;
    if parsed.models.is_empty() {
        return Ok(None);
    }

    let version = parsed
        .version
        .and_then(normalize_pricing_catalog_version)
        .unwrap_or_else(|| DEFAULT_PRICING_CATALOG_VERSION.to_string());
    let models = parsed
        .models
        .into_iter()
        .map(|(model, pricing)| {
            let cache_read_per_1m = pricing.cache_read_per_1m.or(pricing.cache_input_per_1m);
            (
                model,
                ModelPricing {
                    input_per_1m: pricing.input_per_1m,
                    output_per_1m: pricing.output_per_1m,
                    cache_input_per_1m: cache_read_per_1m,
                    cache_read_per_1m,
                    cache_write_per_1m: pricing.cache_write_per_1m,
                    reasoning_per_1m: pricing.reasoning_per_1m,
                    source: pricing
                        .source
                        .map(normalize_pricing_source)
                        .unwrap_or_else(default_pricing_source_custom),
                },
            )
        })
        .collect::<HashMap<_, _>>();

    Ok(Some(PricingCatalog { version, models }))
}

pub(crate) async fn load_pricing_catalog(pool: &Pool<Sqlite>) -> Result<PricingCatalog> {
    seed_default_pricing_catalog(pool).await?;

    let meta = sqlx::query_as::<_, PricingSettingsMetaRow>(
        r#"
        SELECT catalog_version
        FROM pricing_settings_meta
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to load pricing_settings_meta row")?;
    let version = meta
        .map(|row| row.catalog_version)
        .unwrap_or_else(|| DEFAULT_PRICING_CATALOG_VERSION.to_string());

    let rows = sqlx::query_as::<_, PricingSettingsModelRow>(
        r#"
        SELECT
            model,
            input_per_1m,
            output_per_1m,
            cache_input_per_1m,
            cache_read_per_1m,
            cache_write_per_1m,
            reasoning_per_1m,
            source
        FROM pricing_settings_models
        "#,
    )
    .fetch_all(pool)
    .await
    .context("failed to load pricing_settings_models rows")?;

    let mut models = HashMap::new();
    for row in rows {
        // Legacy rows may only populate cache_input_per_1m, but explicit read pricing must win
        // whenever both columns are present.
        let cache_read_per_1m = row.cache_read_per_1m.or(row.cache_input_per_1m);
        models.insert(
            row.model,
            ModelPricing {
                input_per_1m: row.input_per_1m,
                output_per_1m: row.output_per_1m,
                cache_input_per_1m: cache_read_per_1m,
                cache_read_per_1m,
                cache_write_per_1m: row.cache_write_per_1m,
                reasoning_per_1m: row.reasoning_per_1m,
                source: normalize_pricing_source(row.source),
            },
        );
    }

    Ok(PricingCatalog { version, models })
}

pub(crate) async fn save_pricing_catalog(
    pool: &Pool<Sqlite>,
    catalog: &PricingCatalog,
) -> Result<()> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin pricing transaction")?;
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&mut *tx)
        .await
        .context("failed to clear pricing_settings_models rows")?;

    let mut keys = catalog.models.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for model in keys {
        let pricing = catalog
            .models
            .get(&model)
            .with_context(|| format!("missing pricing entry while saving: {model}"))?;
        sqlx::query(
            r#"
            INSERT INTO pricing_settings_models (
                model,
                input_per_1m,
                output_per_1m,
                cache_input_per_1m,
                cache_read_per_1m,
                cache_write_per_1m,
                reasoning_per_1m,
                source,
                updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
            "#,
        )
        .bind(model)
        .bind(pricing.input_per_1m)
        .bind(pricing.output_per_1m)
        .bind(pricing.effective_cache_read_per_1m())
        .bind(pricing.effective_cache_read_per_1m())
        .bind(pricing.cache_write_per_1m)
        .bind(pricing.reasoning_per_1m)
        .bind(&pricing.source)
        .execute(&mut *tx)
        .await
        .context("failed to insert pricing_settings_models row")?;
    }

    sqlx::query(
        r#"
        INSERT INTO pricing_settings_meta (id, catalog_version, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(id) DO UPDATE SET
            catalog_version = excluded.catalog_version,
            updated_at = datetime('now')
        "#,
    )
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .bind(&catalog.version)
    .execute(&mut *tx)
    .await
    .context("failed to upsert pricing_settings_meta row")?;

    tx.commit()
        .await
        .context("failed to commit pricing transaction")?;
    Ok(())
}

pub(crate) fn default_pricing_catalog() -> PricingCatalog {
    let models = [
        (
            "gpt-5.3-codex",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                cache_read_per_1m: Some(0.175),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2-codex",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                cache_read_per_1m: Some(0.175),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-codex-max",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                cache_read_per_1m: Some(0.125),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-codex-mini",
            ModelPricing {
                input_per_1m: 0.25,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.025),
                cache_read_per_1m: Some(0.025),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                cache_read_per_1m: Some(0.175),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.6-sol",
            ModelPricing {
                input_per_1m: 5.0,
                output_per_1m: 30.0,
                cache_input_per_1m: Some(0.5),
                cache_read_per_1m: Some(0.5),
                cache_write_per_1m: Some(6.25),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.6-terra",
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: Some(3.125),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.6-luna",
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 6.0,
                cache_input_per_1m: Some(0.10),
                cache_read_per_1m: Some(0.10),
                cache_write_per_1m: Some(1.25),
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.4",
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                cache_read_per_1m: Some(0.25),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.4-mini",
            ModelPricing {
                input_per_1m: 0.75,
                output_per_1m: 4.5,
                cache_input_per_1m: Some(0.075),
                cache_read_per_1m: Some(0.075),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.5",
            ModelPricing {
                input_per_1m: 5.0,
                output_per_1m: 30.0,
                cache_input_per_1m: Some(0.5),
                cache_read_per_1m: Some(0.5),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                cache_read_per_1m: Some(0.125),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-mini",
            ModelPricing {
                input_per_1m: 0.25,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.025),
                cache_read_per_1m: Some(0.025),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-nano",
            ModelPricing {
                input_per_1m: 0.05,
                output_per_1m: 0.4,
                cache_input_per_1m: Some(0.005),
                cache_read_per_1m: Some(0.005),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2-chat-latest",
            ModelPricing {
                input_per_1m: 1.75,
                output_per_1m: 14.0,
                cache_input_per_1m: Some(0.175),
                cache_read_per_1m: Some(0.175),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-chat-latest",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                cache_read_per_1m: Some(0.125),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-chat-latest",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                cache_read_per_1m: Some(0.125),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.1-codex",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                cache_read_per_1m: Some(0.125),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-codex",
            ModelPricing {
                input_per_1m: 1.25,
                output_per_1m: 10.0,
                cache_input_per_1m: Some(0.125),
                cache_read_per_1m: Some(0.125),
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.2-pro",
            ModelPricing {
                input_per_1m: 21.0,
                output_per_1m: 168.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.4-pro",
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5.5-pro",
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
        (
            "gpt-5-pro",
            ModelPricing {
                input_per_1m: 15.0,
                output_per_1m: 120.0,
                cache_input_per_1m: None,
                cache_read_per_1m: None,
                cache_write_per_1m: None,
                reasoning_per_1m: None,
                source: "official".to_string(),
            },
        ),
    ]
    .into_iter()
    .map(|(model, pricing)| (model.to_string(), pricing))
    .collect::<HashMap<_, _>>();

    PricingCatalog {
        version: DEFAULT_PRICING_CATALOG_VERSION.to_string(),
        models,
    }
}
