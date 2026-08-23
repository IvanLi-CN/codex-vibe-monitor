use super::*;

pub(crate) const MAX_UPSTREAM_ACCOUNT_MODEL_MAPPINGS: usize = 100;
pub(crate) const MAX_WARMED_ROUTING_MODELS: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelMapping {
    pub(crate) source_model: String,
    pub(crate) target_model: String,
    #[serde(default = "default_model_mapping_enabled")]
    pub(crate) enabled: bool,
}

fn default_model_mapping_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedModelMapping {
    pub(crate) source_model: String,
    pub(crate) target_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledModelMapping {
    source_model: String,
    target_model: String,
    source_pattern_ascii_lowercase: String,
    literal_len: usize,
    exact: bool,
}

pub(crate) fn normalize_model_mappings(
    mappings: Vec<ModelMapping>,
) -> Result<Vec<ModelMapping>, (StatusCode, String)> {
    if mappings.len() > MAX_UPSTREAM_ACCOUNT_MODEL_MAPPINGS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "modelMappings must contain at most {MAX_UPSTREAM_ACCOUNT_MODEL_MAPPINGS} entries"
            ),
        ));
    }

    let mut seen_sources = HashSet::with_capacity(mappings.len());
    let mut normalized = Vec::with_capacity(mappings.len());
    for (index, mapping) in mappings.into_iter().enumerate() {
        let source_model = mapping.source_model.trim().to_string();
        let target_model = mapping.target_model.trim().to_string();
        if source_model.is_empty() || target_model.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("modelMappings[{index}] requires non-empty sourceModel and targetModel"),
            ));
        }
        let source_key = source_model.to_ascii_lowercase();
        if !seen_sources.insert(source_key) {
            return Err((
                StatusCode::BAD_REQUEST,
                "modelMappings contains duplicate sourceModel rules".to_string(),
            ));
        }
        normalized.push(ModelMapping {
            source_model,
            target_model,
            enabled: mapping.enabled,
        });
    }
    Ok(normalized)
}

pub(crate) fn decode_model_mappings_json(raw: Option<&str>) -> Vec<ModelMapping> {
    raw.and_then(|value| serde_json::from_str::<Vec<ModelMapping>>(value).ok())
        .and_then(|mappings| normalize_model_mappings(mappings).ok())
        .unwrap_or_default()
}

pub(crate) fn encode_model_mappings_json(mappings: &[ModelMapping]) -> Result<String> {
    Ok(serde_json::to_string(mappings)?)
}

pub(crate) fn compile_model_mappings(mappings: &[ModelMapping]) -> Vec<CompiledModelMapping> {
    mappings
        .iter()
        .filter(|mapping| mapping.enabled)
        .map(|mapping| {
            let source_pattern_ascii_lowercase = mapping.source_model.to_ascii_lowercase();
            CompiledModelMapping {
                source_model: mapping.source_model.clone(),
                target_model: mapping.target_model.clone(),
                literal_len: source_pattern_ascii_lowercase
                    .chars()
                    .filter(|value| *value != '*')
                    .count(),
                exact: !source_pattern_ascii_lowercase.contains('*'),
                source_pattern_ascii_lowercase,
            }
        })
        .collect()
}

pub(crate) fn resolve_model_mapping(
    mappings: &[ModelMapping],
    requested_model: Option<&str>,
) -> Option<ResolvedModelMapping> {
    resolve_compiled_model_mapping(&compile_model_mappings(mappings), requested_model)
}

pub(crate) fn resolve_compiled_model_mapping(
    mappings: &[CompiledModelMapping],
    requested_model: Option<&str>,
) -> Option<ResolvedModelMapping> {
    let requested_model = requested_model?.trim().to_ascii_lowercase();
    let mut best: Option<(usize, &CompiledModelMapping)> = None;
    for (index, mapping) in mappings.iter().enumerate() {
        if !model_mapping_pattern_matches_ascii_lowercase(
            mapping.source_pattern_ascii_lowercase.as_bytes(),
            requested_model.as_bytes(),
        ) {
            continue;
        }
        let replace = best.as_ref().is_none_or(|(best_index, best_mapping)| {
            (mapping.exact && !best_mapping.exact)
                || (mapping.exact == best_mapping.exact
                    && mapping.literal_len > best_mapping.literal_len)
                || (mapping.exact == best_mapping.exact
                    && mapping.literal_len == best_mapping.literal_len
                    && index < *best_index)
        });
        if replace {
            best = Some((index, mapping));
        }
    }
    best.map(|(_, mapping)| ResolvedModelMapping {
        source_model: mapping.source_model.clone(),
        target_model: mapping.target_model.clone(),
    })
}

pub(crate) fn model_mapping_pattern_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    let value = value.trim().to_ascii_lowercase();
    model_mapping_pattern_matches_ascii_lowercase(pattern.as_bytes(), value.as_bytes())
}

fn model_mapping_pattern_matches_ascii_lowercase(pattern: &[u8], value: &[u8]) -> bool {
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star_index = None;
    let mut star_value_index = 0;

    while value_index < value.len() {
        if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_value_index = value_index;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == value[value_index] {
            pattern_index += 1;
            value_index += 1;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            star_value_index += 1;
            value_index = star_value_index;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

pub(crate) fn account_accepts_requested_model_or_mapping(
    requested_model: Option<&str>,
    rule: &EffectiveRoutingRule,
    mappings: &[ModelMapping],
) -> bool {
    if requested_model_is_system_denied(requested_model, rule) {
        return false;
    }

    match resolve_model_mapping(mappings, requested_model) {
        Some(mapping) => mapped_target_model_is_allowed(&mapping.target_model, rule),
        None => account_accepts_requested_model(requested_model, rule),
    }
}

fn mapped_target_model_is_allowed(target_model: &str, rule: &EffectiveRoutingRule) -> bool {
    if requested_model_is_system_denied(Some(target_model), rule) {
        return false;
    }
    rule.tag_available_models
        .as_deref()
        .is_none_or(|allowed_models| {
            allowed_models
                .iter()
                .any(|candidate| requested_model_matches_constraint(target_model, candidate))
        })
}

pub(crate) async fn build_pool_model_routing_runtime_cache(
    pool: &Pool<Sqlite>,
) -> Result<PoolModelRoutingRuntimeCache> {
    build_pool_model_routing_runtime_cache_with_mapping_override(pool, None).await
}

pub(crate) async fn build_pool_model_routing_runtime_cache_with_mapping_override(
    pool: &Pool<Sqlite>,
    mapping_override: Option<(i64, &[ModelMapping])>,
) -> Result<PoolModelRoutingRuntimeCache> {
    let query = format!(
        "SELECT {UPSTREAM_ACCOUNT_ROW_SELECT_COLUMNS} \
         FROM pool_upstream_accounts \
         WHERE COALESCE(deleted_at, '') = '' \
         ORDER BY id ASC"
    );
    let rows = sqlx::query_as::<_, UpstreamAccountRow>(&query)
        .fetch_all(pool)
        .await?;

    let mappings_by_account = rows
        .iter()
        .map(|row| {
            let compiled_mappings = mapping_override
                .as_ref()
                .filter(|(account_id, _)| *account_id == row.id)
                .map(|(_, mappings)| compile_model_mappings(mappings))
                .unwrap_or_else(|| {
                    compile_model_mappings(&decode_model_mappings_json(
                        row.model_mappings_json.as_deref(),
                    ))
                });
            (row.id, compiled_mappings)
        })
        .collect::<HashMap<_, _>>();
    // Sticky ownership and cut-out policy must still be evaluated for a
    // cooling or otherwise non-assignable account. Keep the candidate set
    // selective below, but snapshot routing rules for every live account.
    let routing_account_ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let effective_rules =
        load_effective_routing_rules_for_accounts(pool, &routing_account_ids).await?;
    let mut available_models = Vec::new();
    let mut seen_models = HashSet::new();
    for row in rows.iter().filter(|row| is_routing_eligible_account(row)) {
        let Some(rule) = effective_rules.get(&row.id) else {
            continue;
        };
        if !rule.available_models_defined
            || rule.available_models_mode != AvailableModelsMode::Allowlist
        {
            continue;
        }
        for model in &rule.available_models {
            let model = model.trim();
            if model.is_empty() || model.contains('*') {
                continue;
            }
            let normalized = model.to_ascii_lowercase();
            if seen_models.insert(normalized) {
                available_models.push(model.to_string());
            }
        }
    }

    let mut warmed_model_account_ids = HashMap::new();
    for model in available_models.iter().take(MAX_WARMED_ROUTING_MODELS) {
        let account_ids =
            rows.iter()
                .filter(|row| is_routing_eligible_account(row))
                .filter(|row| {
                    effective_rules.get(&row.id).is_some_and(|rule| {
                        if requested_model_is_system_denied(Some(model), rule) {
                            return false;
                        }
                        match mappings_by_account.get(&row.id).and_then(|mappings| {
                            resolve_compiled_model_mapping(mappings, Some(model))
                        }) {
                            Some(mapping) => {
                                mapped_target_model_is_allowed(&mapping.target_model, rule)
                            }
                            None => account_accepts_requested_model(Some(model), rule),
                        }
                    })
                })
                .map(|row| row.id)
                .collect::<Vec<_>>();
        warmed_model_account_ids.insert(model.to_ascii_lowercase(), account_ids);
    }
    let routing_candidates = load_account_routing_candidates(pool, &HashSet::new()).await?;
    let routing_candidate_ids = routing_candidates
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let group_names = rows
        .iter()
        .filter_map(|row| normalize_optional_text(row.group_name.clone()))
        .collect::<HashSet<_>>();
    let mut group_metadata_by_name = HashMap::with_capacity(group_names.len());
    for group_name in group_names {
        group_metadata_by_name.insert(
            group_name.clone(),
            load_group_metadata(pool, Some(&group_name)).await?,
        );
    }
    let route_binding_failure_penalties = load_recent_route_binding_failure_penalties(pool).await?;
    let transport_decode_sticky_escape_states =
        load_transport_decode_sticky_escape_states(pool, &routing_candidate_ids).await?;
    let model_route_runtime = load_model_route_runtime_snapshots(pool).await?;
    let routing_account_rows_by_id = rows
        .into_iter()
        .map(|row| (row.id, std::sync::Arc::new(row)))
        .collect::<HashMap<_, _>>();

    Ok(PoolModelRoutingRuntimeCache {
        generation: 0,
        mappings_by_account,
        routing_account_rows_by_id,
        routing_candidates,
        effective_rules_by_account: effective_rules,
        group_metadata_by_name,
        route_binding_failure_penalties,
        transport_decode_sticky_escape_states,
        model_route_runtime,
        available_models,
        warmed_model_account_ids,
    })
}

pub(crate) async fn load_model_mapping_for_account(
    state: &AppState,
    account_id: i64,
    requested_model: Option<&str>,
) -> Result<Option<ResolvedModelMapping>> {
    let runtime_cache = load_pool_routing_runtime_cache(state).await?;
    Ok(runtime_cache
        .model_routing
        .mappings_by_account
        .get(&account_id)
        .and_then(|mappings| resolve_compiled_model_mapping(mappings, requested_model)))
}

pub(crate) async fn account_accepts_requested_model_or_cached_mapping(
    state: &AppState,
    account_id: i64,
    requested_model: Option<&str>,
    rule: &EffectiveRoutingRule,
) -> Result<bool> {
    if requested_model_is_system_denied(requested_model, rule) {
        return Ok(false);
    }
    Ok(
        match load_model_mapping_for_account(state, account_id, requested_model).await? {
            Some(mapping) => mapped_target_model_is_allowed(&mapping.target_model, rule),
            None => account_accepts_requested_model(requested_model, rule),
        },
    )
}

pub(crate) async fn account_accepts_requested_model_or_mapping_with_available_models_bypass(
    state: &AppState,
    account_id: i64,
    requested_model: Option<&str>,
    rule: &EffectiveRoutingRule,
) -> Result<bool> {
    if requested_model_is_system_denied(requested_model, rule) {
        return Ok(false);
    }
    Ok(
        match load_model_mapping_for_account(state, account_id, requested_model).await? {
            Some(mapping) => mapped_target_model_is_allowed(&mapping.target_model, rule),
            None => true,
        },
    )
}

pub(crate) async fn install_pool_model_routing_runtime_cache(
    state: &AppState,
    fallback_runtime_cache: PoolRoutingRuntimeCache,
    mut model_routing: PoolModelRoutingRuntimeCache,
) -> PoolModelRoutingRuntimeCache {
    let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
    let runtime_cache = runtime_cache.get_or_insert(fallback_runtime_cache);
    model_routing.generation = runtime_cache.model_routing.generation.saturating_add(1);
    runtime_cache.model_routing = model_routing.clone();
    model_routing
}

pub(crate) async fn refresh_pool_model_routing_runtime_cache(
    state: &AppState,
) -> Result<PoolModelRoutingRuntimeCache> {
    let has_runtime_cache = state.pool_routing_runtime_cache.lock().await.is_some();
    if !has_runtime_cache {
        return Ok(refresh_pool_routing_runtime_cache(state)
            .await?
            .model_routing);
    }

    let _cache_write_guard = state.pool_model_routing_cache_write_lock.lock().await;
    let mut model_routing = build_pool_model_routing_runtime_cache(&state.pool).await?;
    let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
    if let Some(runtime_cache) = runtime_cache.as_mut() {
        model_routing.generation = runtime_cache.model_routing.generation.saturating_add(1);
        runtime_cache.model_routing = model_routing.clone();
        return Ok(model_routing);
    }
    unreachable!("pool routing runtime cache disappeared while refreshing model routing")
}

pub(crate) async fn invalidate_model_mapping_cache_for_account(state: &AppState, account_id: i64) {
    let mut runtime_cache = state.pool_routing_runtime_cache.lock().await;
    if let Some(runtime_cache) = runtime_cache.as_mut() {
        runtime_cache
            .model_routing
            .mappings_by_account
            .remove(&account_id);
    }
}

pub(crate) async fn warmed_routing_account_ids_for_model(
    state: &AppState,
    requested_model: Option<&str>,
) -> Option<Vec<i64>> {
    let model = requested_model?.trim();
    if model.is_empty() {
        return None;
    }
    let runtime_cache = state.pool_routing_runtime_cache.lock().await;
    runtime_cache
        .as_ref()
        .and_then(|cache| {
            cache
                .model_routing
                .warmed_model_account_ids
                .get(&model.to_ascii_lowercase())
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(source_model: &str, target_model: &str, enabled: bool) -> ModelMapping {
        ModelMapping {
            source_model: source_model.to_string(),
            target_model: target_model.to_string(),
            enabled,
        }
    }

    #[test]
    fn model_mapping_matcher_is_ascii_case_insensitive_and_whole_string() {
        assert!(model_mapping_pattern_matches("GPT-*-mini", "gpt-5-MINI"));
        assert!(!model_mapping_pattern_matches("gpt-5", "gpt-5-mini"));
        assert!(model_mapping_pattern_matches(
            "custom?model",
            "custom?MODEL"
        ));
        assert!(!model_mapping_pattern_matches(
            "custom?model",
            "custom-xmodel"
        ));
    }

    #[test]
    fn model_mapping_matcher_prefers_exact_then_literal_length_then_order() {
        let mappings = vec![
            mapping("gpt-*", "first", true),
            mapping("gpt-5-*", "longer", true),
            mapping("gpt-5-mini", "exact", true),
        ];
        assert_eq!(
            resolve_model_mapping(&mappings, Some("GPT-5-MINI"))
                .expect("exact mapping")
                .target_model,
            "exact"
        );
        assert_eq!(
            resolve_model_mapping(&mappings, Some("gpt-5-preview"))
                .expect("longest mapping")
                .target_model,
            "longer"
        );
        let tied = vec![
            mapping("gpt-*-mini", "first", true),
            mapping("gpt-*-mini", "second", false),
        ];
        assert_eq!(
            resolve_model_mapping(&tied, Some("gpt-5-mini"))
                .expect("enabled mapping")
                .target_model,
            "first"
        );

        let unicode_length = vec![
            mapping("*é*", "unicode", true),
            mapping("*aa*", "ascii", true),
        ];
        assert_eq!(
            resolve_model_mapping(&unicode_length, Some("éaa"))
                .expect("unicode mapping")
                .target_model,
            "ascii"
        );
    }

    #[test]
    fn wildcard_mapping_can_match_an_empty_model_but_none_means_no_model() {
        let mappings = vec![mapping("*", "empty-target", true)];
        assert_eq!(
            resolve_model_mapping(&mappings, Some(""))
                .expect("wildcard should match empty model")
                .target_model,
            "empty-target"
        );
        assert_eq!(resolve_model_mapping(&mappings, None), None);
    }

    #[test]
    fn model_mapping_validation_trims_and_rejects_duplicate_sources_including_disabled() {
        let normalized = normalize_model_mappings(vec![mapping(" GPT-* ", " target ", true)])
            .expect("valid mapping");
        assert_eq!(normalized[0].source_model, "GPT-*");
        assert_eq!(normalized[0].target_model, "target");
        let duplicate = normalize_model_mappings(vec![
            mapping("gpt-*", "one", true),
            mapping("GPT-*", "two", false),
        ]);
        assert_eq!(
            duplicate.expect_err("duplicate source").0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn model_mapping_validation_allows_an_empty_list_and_rejects_empty_or_oversized_rows() {
        assert!(
            normalize_model_mappings(Vec::new())
                .expect("empty list is valid")
                .is_empty()
        );
        for mappings in [
            vec![mapping("", "target", true)],
            vec![mapping("source", "   ", true)],
        ] {
            assert_eq!(
                normalize_model_mappings(mappings)
                    .expect_err("empty mapping field must be rejected")
                    .0,
                StatusCode::BAD_REQUEST
            );
        }

        let oversized = (0..=MAX_UPSTREAM_ACCOUNT_MODEL_MAPPINGS)
            .map(|index| mapping(&format!("source-{index}"), "target", true))
            .collect();
        assert_eq!(
            normalize_model_mappings(oversized)
                .expect_err("more than the mapping limit must be rejected")
                .0,
            StatusCode::BAD_REQUEST
        );
    }
}
