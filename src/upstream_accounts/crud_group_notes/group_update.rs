use sqlx::{SqliteConnection, Transaction, query::Query as SqlxQuery, sqlite::SqliteArguments};

const UPDATE_GROUP_ROUTING_RULE_SQL: &str = r#"
UPDATE pool_upstream_account_group_notes
SET policy_allow_cut_out = CASE WHEN ?2 != 0 THEN policy_allow_cut_out ELSE ?3 END,
    policy_allow_cut_in = CASE WHEN ?4 != 0 THEN policy_allow_cut_in ELSE ?5 END,
    policy_priority_tier = CASE WHEN ?6 != 0 THEN policy_priority_tier ELSE ?7 END,
    policy_fast_mode_rewrite_mode = CASE WHEN ?8 != 0 THEN policy_fast_mode_rewrite_mode ELSE ?9 END,
    policy_image_tool_rewrite_mode = CASE WHEN ?10 != 0 THEN policy_image_tool_rewrite_mode ELSE ?11 END,
    policy_request_compression_algorithm = CASE WHEN ?12 != 0 THEN policy_request_compression_algorithm ELSE ?13 END,
    policy_concurrency_limit = CASE WHEN ?14 != 0 THEN policy_concurrency_limit ELSE ?15 END,
    policy_upstream_429_retry_enabled = CASE WHEN ?16 != 0 THEN policy_upstream_429_retry_enabled ELSE ?17 END,
    policy_upstream_429_max_retries = CASE WHEN ?18 != 0 THEN policy_upstream_429_max_retries ELSE ?19 END,
    policy_available_models_json = CASE WHEN ?20 != 0 THEN policy_available_models_json ELSE ?21 END,
    policy_status_change_upstream_http_401 = CASE WHEN ?22 != 0 THEN policy_status_change_upstream_http_401 ELSE ?23 END,
    policy_status_change_upstream_http_402 = CASE WHEN ?24 != 0 THEN policy_status_change_upstream_http_402 ELSE ?25 END,
    policy_status_change_upstream_http_403 = CASE WHEN ?26 != 0 THEN policy_status_change_upstream_http_403 ELSE ?27 END,
    policy_status_change_reauth_required = CASE WHEN ?28 != 0 THEN policy_status_change_reauth_required ELSE ?29 END,
    policy_status_change_upstream_http_429_rate_limit = CASE WHEN ?30 != 0 THEN policy_status_change_upstream_http_429_rate_limit ELSE ?31 END,
    policy_status_change_upstream_http_429_quota_exhausted = CASE WHEN ?32 != 0 THEN policy_status_change_upstream_http_429_quota_exhausted ELSE ?33 END,
    policy_status_change_usage_snapshot_exhausted = CASE WHEN ?34 != 0 THEN policy_status_change_usage_snapshot_exhausted ELSE ?35 END,
    policy_status_change_quota_still_exhausted = CASE WHEN ?36 != 0 THEN policy_status_change_quota_still_exhausted ELSE ?37 END,
    policy_status_change_transport_failure = CASE WHEN ?38 != 0 THEN policy_status_change_transport_failure ELSE ?39 END,
    policy_status_change_upstream_server_overloaded = CASE WHEN ?40 != 0 THEN policy_status_change_upstream_server_overloaded ELSE ?41 END,
    policy_status_change_upstream_http_5xx = CASE WHEN ?42 != 0 THEN policy_status_change_upstream_http_5xx ELSE ?43 END,
    policy_responses_first_byte_timeout_secs = CASE WHEN ?44 != 0 THEN policy_responses_first_byte_timeout_secs ELSE ?45 END,
    policy_compact_first_byte_timeout_secs = CASE WHEN ?46 != 0 THEN policy_compact_first_byte_timeout_secs ELSE ?47 END,
    policy_image_first_byte_timeout_secs = CASE WHEN ?48 != 0 THEN policy_image_first_byte_timeout_secs ELSE ?49 END,
    policy_responses_stream_timeout_secs = CASE WHEN ?50 != 0 THEN policy_responses_stream_timeout_secs ELSE ?51 END,
    policy_compact_stream_timeout_secs = CASE WHEN ?52 != 0 THEN policy_compact_stream_timeout_secs ELSE ?53 END,
    policy_codex_imagegen_rewrite_mode = CASE WHEN ?54 != 0 THEN policy_codex_imagegen_rewrite_mode ELSE ?55 END
WHERE group_name = ?1
"#;

pub(crate) async fn update_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
    Json(payload): Json<UpdateUpstreamAccountGroupRequest>,
) -> Result<Json<UpstreamAccountGroupSummary>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let group_name = normalize_optional_text(Some(group_name)).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "group name is required".to_string(),
        )
    })?;
    let mut tx = state
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(internal_error_tuple)?;
    save_group_metadata_update(state.as_ref(), &mut tx, &group_name, &payload).await?;
    if let Some(rule) = payload.routing_rule.as_ref() {
        apply_group_routing_rule_update(tx.as_mut(), &group_name, rule).await?;
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    publish_group_update(state.as_ref(), &group_name).await;
    load_group_update_summary(state.as_ref(), &group_name).await
}

async fn save_group_metadata_update(
    state: &AppState,
    tx: &mut Transaction<'_, Sqlite>,
    group_name: &str,
    payload: &UpdateUpstreamAccountGroupRequest,
) -> Result<(), (StatusCode, String)> {
    let existing_metadata = load_group_metadata_conn(tx.as_mut(), group_name)
        .await
        .map_err(internal_error_tuple)?
        .unwrap_or_default();
    let metadata = resolve_group_metadata(state, group_name, payload, existing_metadata).await?;
    save_group_metadata_record_conn(tx.as_mut(), group_name, metadata)
        .await
        .map_err(internal_error_tuple)
}

async fn resolve_group_metadata(
    state: &AppState,
    group_name: &str,
    payload: &UpdateUpstreamAccountGroupRequest,
    existing: UpstreamAccountGroupMetadata,
) -> Result<UpstreamAccountGroupMetadata, (StatusCode, String)> {
    let bound_proxy_keys_was_updated = payload.bound_proxy_keys.is_some();
    let mut bound_proxy_keys = payload
        .bound_proxy_keys
        .as_ref()
        .map(|keys| normalize_bound_proxy_keys(keys.clone()))
        .unwrap_or_default();
    if bound_proxy_keys_was_updated {
        bound_proxy_keys = canonicalize_forward_proxy_bound_keys(state, &bound_proxy_keys)
            .await
            .map_err(internal_error_tuple)?;
    }
    let next_bound_proxy_keys = if bound_proxy_keys_was_updated {
        bound_proxy_keys
    } else {
        existing.bound_proxy_keys.clone()
    };
    let node_shunt_updated = payload.node_shunt_enabled.is_some();
    let next_node_shunt_enabled = payload
        .node_shunt_enabled
        .unwrap_or(existing.node_shunt_enabled);
    let node_shunt_was_disabled =
        existing.node_shunt_enabled && node_shunt_updated && !next_node_shunt_enabled;
    validate_group_proxy_bindings(
        state,
        group_name,
        &next_bound_proxy_keys,
        next_node_shunt_enabled,
        bound_proxy_keys_was_updated || node_shunt_was_disabled,
    )
    .await?;
    let normalized_max_retries = payload
        .upstream_429_max_retries
        .map(normalize_group_upstream_429_max_retries)
        .unwrap_or_default();
    let concurrency_limit =
        normalize_concurrency_limit(payload.concurrency_limit.or(Some(0)), "concurrencyLimit")?;
    Ok(UpstreamAccountGroupMetadata {
        note: normalize_optional_text(payload.note.clone()),
        bound_proxy_keys: next_bound_proxy_keys,
        node_shunt_enabled: next_node_shunt_enabled,
        single_account_rotation_enabled: payload
            .single_account_rotation_enabled
            .unwrap_or(existing.single_account_rotation_enabled),
        upstream_429_retry_enabled: payload
            .upstream_429_retry_enabled
            .unwrap_or(existing.upstream_429_retry_enabled),
        upstream_429_max_retries: payload
            .upstream_429_max_retries
            .map(|_| normalized_max_retries)
            .unwrap_or(existing.upstream_429_max_retries),
        concurrency_limit: payload
            .concurrency_limit
            .map(|_| concurrency_limit)
            .unwrap_or(existing.concurrency_limit),
    })
}

async fn validate_group_proxy_bindings(
    state: &AppState,
    group_name: &str,
    bound_proxy_keys: &[String],
    node_shunt_enabled: bool,
    bindings_changed: bool,
) -> Result<(), (StatusCode, String)> {
    if node_shunt_enabled && bound_proxy_keys.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            missing_group_bound_proxy_error_message(group_name.trim()),
        ));
    }
    if !node_shunt_enabled && !bound_proxy_keys.is_empty() && bindings_changed {
        let selectable = state
            .forward_proxy
            .lock()
            .await
            .has_selectable_bound_proxy_keys(bound_proxy_keys);
        if !selectable {
            return Err((
                StatusCode::BAD_REQUEST,
                "select at least one available proxy node or clear bindings before saving"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

async fn publish_group_update(state: &AppState, group_name: &str) {
    if let Err(err) = publish_account_effective_routing_rules_changed(state, None, &[]).await {
        warn!(
            ?err,
            group_name, "group update committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
}

async fn load_group_update_summary(
    state: &AppState,
    group_name: &str,
) -> Result<Json<UpstreamAccountGroupSummary>, (StatusCode, String)> {
    let saved = load_group_metadata(&state.pool, Some(group_name))
        .await
        .map_err(internal_error_tuple)?;
    let mut conn = state.pool.acquire().await.map_err(internal_error_tuple)?;
    let account_count = group_account_count_conn(&mut conn, group_name)
        .await
        .map_err(internal_error_tuple)?;
    let routing_rule = load_group_routing_rule(&state.pool, group_name)
        .await
        .map_err(internal_error_tuple)?
        .clone();
    let (effective_timeouts, timeout_field_sources, _) =
        load_effective_request_path_timeouts_for_group(
            &state.pool,
            &state.config,
            Some(group_name),
        )
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(UpstreamAccountGroupSummary {
        group_name: group_name.to_string(),
        account_count,
        note: saved.note,
        bound_proxy_keys: saved.bound_proxy_keys,
        node_shunt_enabled: saved.node_shunt_enabled,
        single_account_rotation_enabled: saved.single_account_rotation_enabled,
        upstream_429_retry_enabled: saved.upstream_429_retry_enabled,
        upstream_429_max_retries: saved.upstream_429_max_retries,
        concurrency_limit: saved.concurrency_limit,
        routing_rule,
        effective_timeouts,
        timeout_field_sources,
    }))
}

struct NormalizedGroupRoutingRule<'a> {
    rule: &'a UpdateGroupAccountRoutingRuleRequest,
    policy_concurrency_limit: Option<i64>,
    policy_priority_tier: Option<String>,
    policy_fast_mode_rewrite_mode: Option<String>,
    policy_image_tool_rewrite_mode: Option<String>,
    policy_codex_imagegen_rewrite_mode: Option<String>,
    policy_request_compression_algorithm: Option<String>,
    available_models_json: Option<String>,
    available_models_mode: Option<String>,
    preserve_available_models_json: bool,
    status_change: [OptionalField<bool>; 11],
    responses_first_byte_timeout_secs: Option<Option<i64>>,
    compact_first_byte_timeout_secs: Option<Option<i64>>,
    image_first_byte_timeout_secs: Option<Option<i64>>,
    responses_stream_timeout_secs: Option<Option<i64>>,
    compact_stream_timeout_secs: Option<Option<i64>>,
}

async fn apply_group_routing_rule_update(
    conn: &mut SqliteConnection,
    group_name: &str,
    rule: &UpdateGroupAccountRoutingRuleRequest,
) -> Result<(), (StatusCode, String)> {
    let patch = normalize_group_routing_rule(rule)?;
    persist_group_routing_rule(conn, group_name, &patch)
        .await
        .map_err(internal_error_tuple)
}

fn normalize_group_routing_rule<'a>(
    rule: &'a UpdateGroupAccountRoutingRuleRequest,
) -> Result<NormalizedGroupRoutingRule<'a>, (StatusCode, String)> {
    let policy_concurrency_limit = match rule.concurrency_limit {
        OptionalField::Value(value) => Some(normalize_concurrency_limit(
            Some(value),
            "concurrencyLimit",
        )?),
        OptionalField::Missing | OptionalField::Null => None,
    };
    let policy_priority_tier = rule
        .priority_tier_value()
        .map(|value| normalize_tag_priority_tier(Some(value)).map(|tier| tier.as_str().to_owned()))
        .transpose()?;
    let policy_fast_mode_rewrite_mode = rule
        .fast_mode_rewrite_mode_value()
        .map(|value| {
            normalize_tag_fast_mode_rewrite_mode(Some(value)).map(|mode| mode.as_str().to_owned())
        })
        .transpose()?;
    let policy_image_tool_rewrite_mode = rule
        .image_tool_rewrite_mode_value()
        .map(|value| {
            super::sync::normalize_upstream_image_tool_rewrite_mode(Some(value))
                .map(|mode| mode.as_str().to_owned())
        })
        .transpose()?;
    let policy_codex_imagegen_rewrite_mode = rule
        .codex_imagegen_rewrite_mode_value()
        .map(|value| {
            super::sync::normalize_codex_imagegen_rewrite_mode(Some(value))
                .map(|mode| mode.as_str().to_owned())
        })
        .transpose()?;
    let policy_request_compression_algorithm = rule
        .request_compression_algorithm_value()
        .map(|value| {
            normalize_request_compression_algorithm(Some(value))
                .map(|mode| mode.as_str().to_owned())
        })
        .transpose()?;
    let normalized_available_models = normalize_group_available_models(rule)?;
    let timeout_patch = rule.timeouts.clone().unwrap_or_default();
    let responses_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.responses_first_byte_timeout_secs,
        "responsesFirstByteTimeoutSecs",
    )?;
    let compact_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.compact_first_byte_timeout_secs,
        "compactFirstByteTimeoutSecs",
    )?;
    let image_first_byte_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.image_first_byte_timeout_secs,
        "imageFirstByteTimeoutSecs",
    )?;
    let responses_stream_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.responses_stream_timeout_secs,
        "responsesStreamTimeoutSecs",
    )?;
    let compact_stream_timeout_secs = normalize_optional_timeout_override_secs(
        &timeout_patch.compact_stream_timeout_secs,
        "compactStreamTimeoutSecs",
    )?;
    let status_change =
        normalize_group_status_change_reasons(rule).map_err(internal_error_tuple)?;
    Ok(NormalizedGroupRoutingRule {
        rule,
        policy_concurrency_limit,
        policy_priority_tier,
        policy_fast_mode_rewrite_mode,
        policy_image_tool_rewrite_mode,
        policy_codex_imagegen_rewrite_mode,
        policy_request_compression_algorithm,
        available_models_json: normalized_available_models.json,
        available_models_mode: normalized_available_models.mode,
        preserve_available_models_json: normalized_available_models.preserve,
        status_change,
        responses_first_byte_timeout_secs,
        compact_first_byte_timeout_secs,
        image_first_byte_timeout_secs,
        responses_stream_timeout_secs,
        compact_stream_timeout_secs,
    })
}

struct NormalizedGroupAvailableModels {
    json: Option<String>,
    mode: Option<String>,
    preserve: bool,
}

fn normalize_group_available_models(
    rule: &UpdateGroupAccountRoutingRuleRequest,
) -> Result<NormalizedGroupAvailableModels, (StatusCode, String)> {
    let mut available_models_json = match &rule.available_models {
        OptionalField::Missing | OptionalField::Null => None,
        OptionalField::Value(value) => Some(
            encode_string_array_json(&normalize_available_models(
                Some(value.clone()),
                "availableModels",
            )?)
            .map_err(internal_error_tuple)?,
        ),
    };
    let available_models_mode = match &rule.available_models_mode {
        OptionalField::Missing => match &rule.available_models {
            OptionalField::Value(_) => Some("allowlist".to_string()),
            OptionalField::Missing | OptionalField::Null => None,
        },
        OptionalField::Null => None,
        OptionalField::Value(value) => {
            let normalized = value.trim().to_ascii_lowercase();
            if normalized != "allowlist" && normalized != "denylist" {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "availableModelsMode must be allowlist or denylist".to_string(),
                ));
            }
            Some(normalized)
        }
    };
    if matches!(rule.available_models_mode, OptionalField::Null) {
        available_models_json = None;
    }
    let preserve = matches!(rule.available_models, OptionalField::Missing)
        && !matches!(rule.available_models_mode, OptionalField::Null);
    Ok(NormalizedGroupAvailableModels {
        json: available_models_json,
        mode: available_models_mode,
        preserve,
    })
}

fn normalize_group_status_change_reasons(
    rule: &UpdateGroupAccountRoutingRuleRequest,
) -> anyhow::Result<[OptionalField<bool>; 11]> {
    Ok([
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED)?,
        rule.status_change_reason_field(
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
        )?,
        rule.status_change_reason_field(
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
        )?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED)?,
        rule.status_change_reason_field(UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX)?,
    ])
}

async fn persist_group_routing_rule(
    conn: &mut SqliteConnection,
    group_name: &str,
    patch: &NormalizedGroupRoutingRule<'_>,
) -> anyhow::Result<()> {
    let query = bind_group_routing_rule_policy_fields(
        sqlx::query(UPDATE_GROUP_ROUTING_RULE_SQL),
        group_name,
        patch,
    );
    let query = bind_group_routing_rule_status_fields(query, patch);
    query.execute(&mut *conn).await?;
    persist_available_models_mode(conn, group_name, patch).await
}

type SqliteRoutingRuleQuery<'query> = SqlxQuery<'query, Sqlite, SqliteArguments<'query>>;

fn bind_group_routing_rule_policy_fields<'query, 'rule>(
    query: SqliteRoutingRuleQuery<'query>,
    group_name: &'query str,
    patch: &'query NormalizedGroupRoutingRule<'rule>,
) -> SqliteRoutingRuleQuery<'query>
where
    'rule: 'query,
{
    let rule = patch.rule;
    query
        .bind(group_name)
        .bind(if matches!(rule.allow_cut_out, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&rule.allow_cut_out))
        .bind(if matches!(rule.allow_cut_in, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&rule.allow_cut_in))
        .bind(if matches!(rule.priority_tier, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.policy_priority_tier.as_deref())
        .bind(
            if matches!(rule.fast_mode_rewrite_mode, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(patch.policy_fast_mode_rewrite_mode.as_deref())
        .bind(
            if matches!(rule.image_tool_rewrite_mode, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(patch.policy_image_tool_rewrite_mode.as_deref())
        .bind(
            if matches!(rule.request_compression_algorithm, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(patch.policy_request_compression_algorithm.as_deref())
        .bind(
            if matches!(rule.concurrency_limit, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(patch.policy_concurrency_limit)
        .bind(
            if matches!(rule.upstream_429_retry_enabled, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(optional_bool_to_i64(&rule.upstream_429_retry_enabled))
        .bind(
            if matches!(rule.upstream_429_max_retries, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(optional_retry_count_to_i64(&rule.upstream_429_max_retries))
        .bind(if patch.preserve_available_models_json {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.available_models_json.as_deref())
}

fn bind_group_routing_rule_status_fields<'query, 'rule>(
    query: SqliteRoutingRuleQuery<'query>,
    patch: &'query NormalizedGroupRoutingRule<'rule>,
) -> SqliteRoutingRuleQuery<'query>
where
    'rule: 'query,
{
    let query = bind_group_routing_rule_status_change_fields(query, patch);
    bind_group_routing_rule_timeout_fields(query, patch)
}

fn bind_group_routing_rule_status_change_fields<'query, 'rule>(
    query: SqliteRoutingRuleQuery<'query>,
    patch: &'query NormalizedGroupRoutingRule<'rule>,
) -> SqliteRoutingRuleQuery<'query>
where
    'rule: 'query,
{
    let [
        status_401,
        status_402,
        status_403,
        status_reauth,
        status_429_rate,
        status_429_quota,
        status_usage,
        status_quota,
        status_transport,
        status_overloaded,
        status_5xx,
    ] = patch.status_change.clone();
    query
        .bind(if matches!(status_401, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_401))
        .bind(if matches!(status_402, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_402))
        .bind(if matches!(status_403, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_403))
        .bind(if matches!(status_reauth, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_reauth))
        .bind(if matches!(status_429_rate, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_429_rate))
        .bind(if matches!(status_429_quota, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_429_quota))
        .bind(if matches!(status_usage, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_usage))
        .bind(if matches!(status_quota, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_quota))
        .bind(if matches!(status_transport, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_transport))
        .bind(if matches!(status_overloaded, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_overloaded))
        .bind(if matches!(status_5xx, OptionalField::Missing) {
            1_i64
        } else {
            0_i64
        })
        .bind(optional_bool_to_i64(&status_5xx))
}

fn bind_group_routing_rule_timeout_fields<'query, 'rule>(
    query: SqliteRoutingRuleQuery<'query>,
    patch: &'query NormalizedGroupRoutingRule<'rule>,
) -> SqliteRoutingRuleQuery<'query>
where
    'rule: 'query,
{
    let rule = patch.rule;
    query
        .bind(if patch.responses_first_byte_timeout_secs.is_none() {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.responses_first_byte_timeout_secs.flatten())
        .bind(if patch.compact_first_byte_timeout_secs.is_none() {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.compact_first_byte_timeout_secs.flatten())
        .bind(if patch.image_first_byte_timeout_secs.is_none() {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.image_first_byte_timeout_secs.flatten())
        .bind(if patch.responses_stream_timeout_secs.is_none() {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.responses_stream_timeout_secs.flatten())
        .bind(if patch.compact_stream_timeout_secs.is_none() {
            1_i64
        } else {
            0_i64
        })
        .bind(patch.compact_stream_timeout_secs.flatten())
        .bind(
            if matches!(rule.codex_imagegen_rewrite_mode, OptionalField::Missing) {
                1_i64
            } else {
                0_i64
            },
        )
        .bind(patch.policy_codex_imagegen_rewrite_mode.as_deref())
}

async fn persist_available_models_mode(
    conn: &mut SqliteConnection,
    group_name: &str,
    patch: &NormalizedGroupRoutingRule<'_>,
) -> anyhow::Result<()> {
    let rule = patch.rule;
    if !matches!(rule.available_models_mode, OptionalField::Missing)
        || matches!(
            rule.available_models,
            OptionalField::Value(_) | OptionalField::Null
        )
    {
        sqlx::query(
            "UPDATE pool_upstream_account_group_notes SET policy_available_models_mode = ?2 WHERE group_name = ?1",
        )
        .bind(group_name)
        .bind(patch.available_models_mode.as_deref())
        .execute(conn)
        .await?;
    }
    Ok(())
}
