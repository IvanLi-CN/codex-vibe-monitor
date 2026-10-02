use super::*;
use crate::models_dev_sync_memory::{
    ModelSelectionChange, ModelsDevSyncMemoryState, ProviderSelectionChange,
    QuoteProviderChoiceChange, load_models_dev_sync_memory, patch_models_dev_sync_memory,
    record_catalog_success,
};
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};

fn provider(id: &str) -> ModelsDevSyncProvider {
    ModelsDevSyncProvider {
        id: id.to_string(),
        name: id.to_string(),
        doc_url: None,
    }
}

fn candidate(model: &str, provider_id: &str) -> ModelsDevPriceCandidate {
    ModelsDevPriceCandidate {
        model: model.to_string(),
        name: model.to_string(),
        provider_id: provider_id.to_string(),
        provider_name: provider_id.to_string(),
        doc_url: None,
        status: None,
        input_per_1m: Some(1.0),
        output_per_1m: Some(2.0),
        cache_read_per_1m: None,
        cache_write_per_1m: None,
        reasoning_per_1m: None,
        unsupported_dimensions: Vec::new(),
        importable: true,
    }
}

#[tokio::test]
async fn sync_memory_initializes_catalog_baseline_and_persists_sparse_choices() {
    let pool = test_current_schema_pool().await;
    let initial = record_catalog_success(
        &pool,
        &[provider("provider-a"), provider("provider-b")],
        &[candidate("model-existing", "provider-a")],
    )
    .await
    .expect("initialize first successful catalog");

    assert!(initial.catalog_baseline_initialized);
    assert!(initial.provider_selection_initialized);
    assert!(initial.unviewed_model_ids.is_empty());
    assert_eq!(
        initial.provider_selections,
        [
            ProviderSelectionChange {
                provider_id: "provider-a".to_string(),
                selected: true,
            },
            ProviderSelectionChange {
                provider_id: "provider-b".to_string(),
                selected: true,
            }
        ]
    );

    let discovered = record_catalog_success(
        &pool,
        &[
            provider("provider-a"),
            provider("provider-b"),
            provider("provider-c"),
        ],
        &[
            candidate("model-existing", "provider-b"),
            candidate("model-new", "provider-c"),
        ],
    )
    .await
    .expect("record later catalog");
    assert_eq!(discovered.unviewed_model_ids, ["model-new"]);
    assert!(
        !discovered
            .provider_selections
            .iter()
            .any(|selection| selection.provider_id == "provider-c")
    );

    let patch = crate::api::ModelsDevSyncMemoryPatch {
        provider_selections: vec![ProviderSelectionChange {
            provider_id: "provider-a".to_string(),
            selected: false,
        }],
        model_selections: vec![
            ModelSelectionChange {
                model: "model-existing".to_string(),
                provider_id: "provider-a".to_string(),
                selected: false,
            },
            ModelSelectionChange {
                model: "model-existing".to_string(),
                provider_id: "provider-b".to_string(),
                selected: true,
            },
        ],
        quote_provider_choices: vec![QuoteProviderChoiceChange {
            model: "model-existing".to_string(),
            provider_id: "provider-b".to_string(),
        }],
        viewed_model_ids: vec!["model-new".to_string()],
    };
    let acknowledged = patch_models_dev_sync_memory(&pool, patch)
        .await
        .expect("save sparse user choices");
    assert!(acknowledged.unviewed_model_ids.is_empty());

    let reloaded = load_models_dev_sync_memory(&pool)
        .await
        .expect("reload sync memory");
    assert!(reloaded.unviewed_model_ids.is_empty());
    assert!(
        reloaded
            .model_selections
            .iter()
            .any(|choice| choice.provider_id == "provider-a" && !choice.selected)
    );
    assert!(
        reloaded
            .model_selections
            .iter()
            .any(|choice| choice.provider_id == "provider-b" && choice.selected)
    );
    assert_eq!(
        reloaded.quote_provider_choices,
        [QuoteProviderChoiceChange {
            model: "model-existing".to_string(),
            provider_id: "provider-b".to_string(),
        }]
    );

    ensure_schema(&pool)
        .await
        .expect("repeat schema initialization");
    let after_restart = load_models_dev_sync_memory(&pool)
        .await
        .expect("reload memory after schema initialization");
    assert_eq!(after_restart, reloaded);
}

#[tokio::test]
async fn sync_memory_concurrent_patches_preserve_choices_and_view_acknowledgments() {
    let pool = test_current_schema_pool().await;
    record_catalog_success(
        &pool,
        &[provider("provider-a")],
        &[candidate("model-a", "provider-a")],
    )
    .await
    .expect("initialize catalog");

    let first = crate::api::ModelsDevSyncMemoryPatch {
        provider_selections: vec![ProviderSelectionChange {
            provider_id: "provider-a".to_string(),
            selected: false,
        }],
        ..Default::default()
    };
    let second = crate::api::ModelsDevSyncMemoryPatch {
        viewed_model_ids: vec!["model-a".to_string()],
        ..Default::default()
    };
    let (first_result, second_result) = tokio::join!(
        patch_models_dev_sync_memory(&pool, first),
        patch_models_dev_sync_memory(&pool, second),
    );
    first_result.expect("save provider choice");
    second_result.expect("acknowledge the model as viewed");

    let same_key_true = crate::api::ModelsDevSyncMemoryPatch {
        model_selections: vec![ModelSelectionChange {
            model: "model-a".to_string(),
            provider_id: "provider-a".to_string(),
            selected: true,
        }],
        ..Default::default()
    };
    let same_key_false = crate::api::ModelsDevSyncMemoryPatch {
        model_selections: vec![ModelSelectionChange {
            model: "model-a".to_string(),
            provider_id: "provider-a".to_string(),
            selected: false,
        }],
        ..Default::default()
    };
    let (true_result, false_result) = tokio::join!(
        patch_models_dev_sync_memory(&pool, same_key_true),
        patch_models_dev_sync_memory(&pool, same_key_false),
    );
    true_result.expect("concurrently select the model");
    false_result.expect("concurrently clear the model selection");

    let after_same_key_race = load_models_dev_sync_memory(&pool)
        .await
        .expect("load same-key concurrent state");
    assert_eq!(after_same_key_race.model_selections.len(), 1);
    assert!(after_same_key_race.unviewed_model_ids.is_empty());

    let same_key = crate::api::ModelsDevSyncMemoryPatch {
        model_selections: vec![ModelSelectionChange {
            model: "model-a".to_string(),
            provider_id: "provider-a".to_string(),
            selected: false,
        }],
        ..Default::default()
    };
    patch_models_dev_sync_memory(&pool, same_key)
        .await
        .expect("apply last write to model choice");

    let state = load_models_dev_sync_memory(&pool)
        .await
        .expect("load concurrently updated state");
    assert!(!state.provider_selections[0].selected);
    assert!(!state.model_selections[0].selected);
    assert!(state.unviewed_model_ids.is_empty());
    assert!(state.catalog_baseline_initialized);
}

#[tokio::test]
async fn sync_memory_patch_rejects_blank_keys_without_writing() {
    let pool = test_current_schema_pool().await;
    let patch = crate::api::ModelsDevSyncMemoryPatch {
        model_selections: vec![ModelSelectionChange {
            model: "model-a".to_string(),
            provider_id: "  ".to_string(),
            selected: true,
        }],
        ..Default::default()
    };
    assert!(patch_models_dev_sync_memory(&pool, patch).await.is_err());
    assert_eq!(
        load_models_dev_sync_memory(&pool)
            .await
            .expect("load unchanged state"),
        ModelsDevSyncMemoryState::default()
    );
}

#[tokio::test]
async fn sync_memory_settings_api_shares_state_and_rejects_cross_origin_writes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let before = crate::api::get_models_sync_state(State(state.clone()))
        .await
        .expect("read initial client state")
        .0;

    let mut cross_origin_headers = HeaderMap::new();
    cross_origin_headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
    let forbidden = crate::api::patch_models_sync_state(
        State(state.clone()),
        cross_origin_headers,
        Json(crate::api::ModelsDevSyncMemoryPatch {
            provider_selections: vec![ProviderSelectionChange {
                provider_id: "provider-a".to_string(),
                selected: true,
            }],
            ..Default::default()
        }),
    )
    .await
    .expect_err("reject a cross-origin browser write");
    assert_eq!(forbidden.0, StatusCode::FORBIDDEN);

    let second_client_before = crate::api::get_models_sync_state(State(state.clone()))
        .await
        .expect("second client reads unchanged state")
        .0;
    assert_eq!(second_client_before, before);

    let saved = crate::api::patch_models_sync_state(
        State(state.clone()),
        HeaderMap::new(),
        Json(crate::api::ModelsDevSyncMemoryPatch {
            provider_selections: vec![ProviderSelectionChange {
                provider_id: "provider-a".to_string(),
                selected: false,
            }],
            model_selections: vec![ModelSelectionChange {
                model: "model-a".to_string(),
                provider_id: "provider-a".to_string(),
                selected: true,
            }],
            ..Default::default()
        }),
    )
    .await
    .expect("save a same-origin sync-memory update")
    .0;
    assert!(!saved.provider_selections[0].selected);
    assert!(saved.model_selections[0].selected);

    let second_client_after = crate::api::get_models_sync_state(State(state))
        .await
        .expect("second client reads saved state")
        .0;
    assert_eq!(second_client_after, saved);
}
