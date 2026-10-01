use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelsDevSyncMemoryState {
    pub(crate) catalog_baseline_initialized: bool,
    pub(crate) provider_selection_initialized: bool,
    pub(crate) provider_selections: Vec<ProviderSelectionChange>,
    pub(crate) model_selections: Vec<ModelSelectionChange>,
    pub(crate) quote_provider_choices: Vec<QuoteProviderChoiceChange>,
    pub(crate) unviewed_model_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderSelectionChange {
    pub(crate) provider_id: String,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelSelectionChange {
    pub(crate) model: String,
    pub(crate) provider_id: String,
    pub(crate) selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct QuoteProviderChoiceChange {
    pub(crate) model: String,
    pub(crate) provider_id: String,
}

pub(crate) async fn ensure_models_dev_sync_memory(pool: &Pool<Sqlite>) -> Result<()> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin models.dev sync-memory schema setup")?;

    for statement in [
        "CREATE TABLE IF NOT EXISTS models_dev_sync_metadata (id INTEGER PRIMARY KEY CHECK (id = 1), catalog_baseline_initialized INTEGER NOT NULL DEFAULT 0, provider_selection_initialized INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE IF NOT EXISTS models_dev_sync_provider_selections (provider_id TEXT PRIMARY KEY, selected INTEGER NOT NULL CHECK (selected IN (0, 1)))",
        "CREATE TABLE IF NOT EXISTS models_dev_sync_model_selections (model TEXT NOT NULL, provider_id TEXT NOT NULL, selected INTEGER NOT NULL CHECK (selected IN (0, 1)), PRIMARY KEY (model, provider_id))",
        "CREATE TABLE IF NOT EXISTS models_dev_sync_quote_provider_choices (model TEXT PRIMARY KEY, provider_id TEXT NOT NULL)",
        "CREATE TABLE IF NOT EXISTS models_dev_sync_discovered_models (model TEXT PRIMARY KEY, first_seen_at TEXT NOT NULL DEFAULT (datetime('now')), viewed_at TEXT)",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .context("failed to create models.dev sync-memory table")?;
    }

    sqlx::query("INSERT OR IGNORE INTO models_dev_sync_metadata (id) VALUES (1)")
        .execute(&mut *tx)
        .await
        .context("failed to initialize models.dev sync-memory metadata")?;

    tx.commit()
        .await
        .context("failed to commit models.dev sync-memory schema setup")
}

pub(crate) async fn load_models_dev_sync_memory(
    pool: &Pool<Sqlite>,
) -> Result<ModelsDevSyncMemoryState> {
    let mut tx = pool
        .begin()
        .await
        .context("failed to begin models.dev sync-memory read")?;
    let state = load_models_dev_sync_memory_tx(&mut tx).await?;
    tx.commit()
        .await
        .context("failed to finish models.dev sync-memory read")?;
    Ok(state)
}

pub(crate) fn validate_patch_keys(patch: &crate::api::ModelsDevSyncMemoryPatch) -> Result<()> {
    validate_key_values(
        patch
            .provider_selections
            .iter()
            .map(|change| change.provider_id.as_str()),
    )?;
    validate_key_values(
        patch
            .model_selections
            .iter()
            .flat_map(|change| [change.model.as_str(), change.provider_id.as_str()]),
    )?;
    validate_key_values(
        patch
            .quote_provider_choices
            .iter()
            .flat_map(|change| [change.model.as_str(), change.provider_id.as_str()]),
    )?;
    validate_key_values(patch.viewed_model_ids.iter().map(String::as_str))?;
    Ok(())
}

pub(crate) async fn patch_models_dev_sync_memory(
    pool: &Pool<Sqlite>,
    patch: crate::api::ModelsDevSyncMemoryPatch,
) -> Result<ModelsDevSyncMemoryState> {
    validate_patch_keys(&patch)?;

    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin models.dev sync-memory update")?;

    for change in patch.provider_selections {
        sqlx::query(
            "INSERT INTO models_dev_sync_provider_selections (provider_id, selected) VALUES (?1, ?2) ON CONFLICT(provider_id) DO UPDATE SET selected = excluded.selected",
        )
        .bind(change.provider_id)
        .bind(i64::from(change.selected))
        .execute(&mut *tx)
        .await
        .context("failed to save models.dev provider selection")?;
    }

    for change in patch.model_selections {
        sqlx::query(
            "INSERT INTO models_dev_sync_model_selections (model, provider_id, selected) VALUES (?1, ?2, ?3) ON CONFLICT(model, provider_id) DO UPDATE SET selected = excluded.selected",
        )
        .bind(change.model)
        .bind(change.provider_id)
        .bind(i64::from(change.selected))
        .execute(&mut *tx)
        .await
        .context("failed to save models.dev model selection")?;
    }

    for change in patch.quote_provider_choices {
        sqlx::query(
            "INSERT INTO models_dev_sync_quote_provider_choices (model, provider_id) VALUES (?1, ?2) ON CONFLICT(model) DO UPDATE SET provider_id = excluded.provider_id",
        )
        .bind(change.model)
        .bind(change.provider_id)
        .execute(&mut *tx)
        .await
        .context("failed to save models.dev quote-provider choice")?;
    }

    for model in patch.viewed_model_ids {
        sqlx::query(
            "UPDATE models_dev_sync_discovered_models SET viewed_at = COALESCE(viewed_at, datetime('now')) WHERE model = ?1",
        )
        .bind(model)
        .execute(&mut *tx)
        .await
        .context("failed to acknowledge a viewed models.dev model")?;
    }

    let state = load_models_dev_sync_memory_tx(&mut tx).await?;
    tx.commit()
        .await
        .context("failed to commit models.dev sync-memory update")?;
    Ok(state)
}

pub(crate) async fn record_catalog_success(
    pool: &Pool<Sqlite>,
    providers: &[crate::api::ModelsDevSyncProvider],
    candidates: &[crate::api::ModelsDevPriceCandidate],
) -> Result<ModelsDevSyncMemoryState> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin models.dev discovery update")?;
    let state = load_metadata(&mut tx).await?;

    if !state.catalog_baseline_initialized {
        for model in candidates
            .iter()
            .map(|candidate| candidate.model.as_str())
            .collect::<BTreeSet<_>>()
        {
            sqlx::query(
                "INSERT OR IGNORE INTO models_dev_sync_discovered_models (model, viewed_at) VALUES (?1, datetime('now'))",
            )
            .bind(model)
            .execute(&mut *tx)
            .await
            .context("failed to seed models.dev discovery baseline")?;
        }
        sqlx::query(
            "UPDATE models_dev_sync_metadata SET catalog_baseline_initialized = 1 WHERE id = 1",
        )
        .execute(&mut *tx)
        .await
        .context("failed to mark models.dev discovery baseline initialized")?;
    } else {
        for model in candidates
            .iter()
            .map(|candidate| candidate.model.as_str())
            .collect::<BTreeSet<_>>()
        {
            sqlx::query(
                "INSERT OR IGNORE INTO models_dev_sync_discovered_models (model) VALUES (?1)",
            )
            .bind(model)
            .execute(&mut *tx)
            .await
            .context("failed to record a newly discovered models.dev model")?;
        }
    }

    if !state.provider_selection_initialized {
        for provider_id in providers
            .iter()
            .map(|provider| provider.id.as_str())
            .collect::<BTreeSet<_>>()
        {
            sqlx::query(
                "INSERT OR IGNORE INTO models_dev_sync_provider_selections (provider_id, selected) VALUES (?1, 1)",
            )
            .bind(provider_id)
            .execute(&mut *tx)
            .await
            .context("failed to initialize models.dev provider selection")?;
        }
        sqlx::query(
            "UPDATE models_dev_sync_metadata SET provider_selection_initialized = 1 WHERE id = 1",
        )
        .execute(&mut *tx)
        .await
        .context("failed to mark models.dev provider selection initialized")?;
    }

    let updated_state = load_models_dev_sync_memory_tx(&mut tx).await?;
    tx.commit()
        .await
        .context("failed to commit models.dev discovery update")?;
    Ok(updated_state)
}

async fn load_models_dev_sync_memory_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
) -> Result<ModelsDevSyncMemoryState> {
    let mut state = load_metadata(tx).await?;
    state.provider_selections = sqlx::query_as::<_, (String, i64)>(
        "SELECT provider_id, selected FROM models_dev_sync_provider_selections ORDER BY provider_id",
    )
    .fetch_all(&mut **tx)
    .await
    .context("failed to load models.dev provider selections")?
    .into_iter()
    .map(|(provider_id, selected)| ProviderSelectionChange {
        provider_id,
        selected: selected != 0,
    })
    .collect();
    state.model_selections = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT model, provider_id, selected FROM models_dev_sync_model_selections ORDER BY model, provider_id",
    )
    .fetch_all(&mut **tx)
    .await
    .context("failed to load models.dev model selections")?
    .into_iter()
    .map(|(model, provider_id, selected)| ModelSelectionChange {
        model,
        provider_id,
        selected: selected != 0,
    })
    .collect();
    state.quote_provider_choices = sqlx::query_as::<_, (String, String)>(
        "SELECT model, provider_id FROM models_dev_sync_quote_provider_choices ORDER BY model",
    )
    .fetch_all(&mut **tx)
    .await
    .context("failed to load models.dev quote-provider choices")?
    .into_iter()
    .map(|(model, provider_id)| QuoteProviderChoiceChange { model, provider_id })
    .collect();
    state.unviewed_model_ids = sqlx::query_scalar::<_, String>(
        "SELECT model FROM models_dev_sync_discovered_models WHERE viewed_at IS NULL ORDER BY model",
    )
    .fetch_all(&mut **tx)
    .await
    .context("failed to load unviewed models.dev model IDs")?;
    Ok(state)
}

async fn load_metadata(tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<ModelsDevSyncMemoryState> {
    let row = sqlx::query_as::<_, (i64, i64)>(
        "SELECT catalog_baseline_initialized, provider_selection_initialized FROM models_dev_sync_metadata WHERE id = 1",
    )
    .fetch_one(&mut **tx)
    .await
    .context("failed to load models.dev sync-memory metadata")?;
    Ok(ModelsDevSyncMemoryState {
        catalog_baseline_initialized: row.0 != 0,
        provider_selection_initialized: row.1 != 0,
        ..ModelsDevSyncMemoryState::default()
    })
}

fn validate_key_values<'a>(values: impl IntoIterator<Item = &'a str>) -> Result<()> {
    if values
        .into_iter()
        .any(|value| value.trim().is_empty() || value.len() > 512)
    {
        anyhow::bail!("sync memory keys must be non-empty and at most 512 bytes")
    }
    Ok(())
}
