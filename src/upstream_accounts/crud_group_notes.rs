use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, QueryBuilder, Sqlite};
use std::{collections::BTreeMap, time::Instant};

include!("crud_group_notes/filtering.rs");
include!("crud_group_notes/attempt_page.rs");
include!("crud_group_notes/action_events.rs");
include!("crud_group_notes/group_update.rs");

include!("crud_group_notes/model_routing.rs");
include!("crud_group_notes/account_attempts.rs");
include!("crud_group_notes/account_management.rs");
const ACCOUNT_ATTEMPT_RETENTION_DAYS: u64 = 7;
const ACCOUNT_ATTEMPT_STICKY_KEY_UNBOUND: &str = "__unbound__";
const ACCOUNT_ATTEMPT_TYPE_NORMAL: &str = "normal";
const ACCOUNT_ATTEMPT_TYPE_REMOTE_V2: &str = "remote_v2";
const ACCOUNT_ATTEMPT_TYPE_IMAGE: &str = "image";
const ACCOUNT_ATTEMPT_TYPE_COMPACT: &str = "compact";
const ACCOUNT_ATTEMPT_RESPONSES_ENDPOINT: &str = "/v1/responses";
const ACCOUNT_ATTEMPT_COMPACT_ENDPOINT: &str = "/v1/responses/compact";
const ACCOUNT_ATTEMPT_IMAGE_ENDPOINT_PREFIX: &str = "/v1/images/%";
const ACCOUNT_ATTEMPT_MODEL_SQL: &str = r#"
COALESCE(
    CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.requestModel') AS TEXT) END,
    inv.model,
    CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.responseModel') AS TEXT) END
)
"#;
const ACCOUNT_ATTEMPT_REQUEST_MODEL_SQL: &str = r#"
COALESCE(
    CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.requestModel') AS TEXT) END,
    inv.model
)
"#;
const ACCOUNT_ATTEMPT_RESPONSE_MODEL_SQL: &str = r#"CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.responseModel') AS TEXT) END"#;
const ACCOUNT_ATTEMPT_COMPACTION_REQUEST_KIND_SQL: &str = r#"CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.compactionRequestKind') AS TEXT) END"#;
const ACCOUNT_ATTEMPT_COMPACTION_RESPONSE_KIND_SQL: &str = r#"CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.compactionResponseKind') AS TEXT) END"#;
const ACCOUNT_ATTEMPT_IMAGE_INTENT_SQL: &str = r#"CASE WHEN json_valid(inv.payload) THEN CAST(json_extract(inv.payload, '$.imageIntent') AS TEXT) END"#;
const MODEL_ROUTING_HISTORY_HOURS: i64 = 48;
const MODEL_ROUTING_LIVE_DEFAULT_LIMIT: usize = 100;
const MODEL_ROUTING_HISTORY_DEFAULT_PAGE_SIZE: usize = 50;
const MODEL_ROUTING_MAX_PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, Default)]
struct UpstreamAccountAttemptFilters {
    attempt_type: Option<String>,
    model: Option<String>,
    sticky_key: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct ModelRoutingAttemptRow {
    id: i64,
    attempt_id: String,
    invoke_id: String,
    occurred_at: String,
    occurred_epoch_ms: i64,
    routing_source: Option<String>,
    routing_selection_audit_json: Option<String>,
    account_id: i64,
    model: String,
    attempt_index: i64,
    same_account_retry_index: i64,
    status: String,
    http_status: Option<i64>,
    failure_kind: Option<String>,
    connect_latency_ms: Option<f64>,
    first_byte_latency_ms: Option<f64>,
    stream_latency_ms: Option<f64>,
    event_action: Option<String>,
    event_source: Option<String>,
    event_reason_code: Option<String>,
    event_state_before: Option<String>,
    event_state_after: Option<String>,
    event_priority_before: Option<String>,
    event_priority_after: Option<String>,
    event_failure_count: Option<i64>,
    event_cooldown_until: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
struct ModelRoutingEventRow {
    id: i64,
    occurred_at: String,
    occurred_epoch_ms: i64,
    account_id: i64,
    model: String,
    action: String,
    source: String,
    result: Option<String>,
    http_status: Option<i64>,
    failure_kind: Option<String>,
    reason_code: Option<String>,
    model_route_state_before: Option<String>,
    model_route_state_after: Option<String>,
    model_route_priority_before: Option<String>,
    model_route_priority_after: Option<String>,
    model_route_failure_count: Option<i64>,
    model_route_cooldown_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRoutingHistoryCursor {
    occurred_epoch_ms: i64,
    kind_rank: i64,
    id: i64,
}

#[derive(Debug, Clone)]
struct ModelRoutingTimelineEntry {
    occurred_epoch_ms: i64,
    kind_rank: i64,
    id: i64,
    record: ModelRoutingTimelineRecord,
}

fn normalize_optional_search_filter(value: Option<&str>) -> Option<String> {
    let normalized = value?.trim();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized.to_string())
    }
}

fn normalize_optional_exact_filter(value: Option<&str>) -> Option<String> {
    normalize_optional_search_filter(value)
}

fn normalize_upstream_account_action_event_result_filter(
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(normalized) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let lower = normalized.to_ascii_lowercase();
    match lower.as_str() {
        "success" | "failed" | "deferred" => Ok(Some(lower)),
        other => Err(format!(
            "invalid result value `{other}`; expected success, failed, or deferred"
        )),
    }
}

pub(crate) async fn list_tags(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListTagsQuery>,
) -> Result<Json<TagListResponse>, (StatusCode, String)> {
    let items = load_tag_summaries(&state.pool, &params)
        .await
        .map_err(internal_error_tuple)?;
    Ok(Json(TagListResponse {
        writes_enabled: false,
        items,
    }))
}

pub(crate) async fn create_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(payload): Json<CreateTagRequest>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let name = normalize_tag_name(&payload.name)?;
    let rule = normalize_tag_rule(
        payload.allow_cut_out,
        payload.allow_cut_in,
        payload.priority_tier.as_deref(),
        payload.fast_mode_rewrite_mode.as_deref(),
        payload.concurrency_limit,
        payload.upstream_429_retry_enabled,
        payload.upstream_429_max_retries,
        Some(payload.available_models),
    )?;
    let detail = insert_tag(&state.pool, &name, &rule)
        .await
        .map_err(map_tag_write_error)?;
    if let Err(err) =
        publish_account_effective_routing_rules_changed(state.as_ref(), None, &[]).await
    {
        warn!(?err, "tag create committed but routing publication failed");
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
    Ok(Json(detail))
}

pub(crate) async fn get_tag(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    let detail = load_tag_detail(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    Ok(Json(detail))
}

pub(crate) async fn update_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<UpdateTagRequest>,
) -> Result<Json<TagDetail>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    let existing = load_tag_row(&state.pool, id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "tag not found".to_string()))?;
    if existing.protected != 0 || existing.system_key.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "system tag cannot be edited".to_string(),
        ));
    }
    let name = match payload.name {
        Some(value) => normalize_tag_name(&value)?,
        None => existing.name.clone(),
    };
    let rule = normalize_tag_rule(
        payload.allow_cut_out.unwrap_or(existing.allow_cut_out != 0),
        payload.allow_cut_in.unwrap_or(existing.allow_cut_in != 0),
        payload
            .priority_tier
            .as_deref()
            .or(Some(existing.priority_tier.as_str())),
        payload
            .fast_mode_rewrite_mode
            .as_deref()
            .or(Some(existing.fast_mode_rewrite_mode.as_str())),
        payload
            .concurrency_limit
            .or(Some(existing.concurrency_limit)),
        payload
            .upstream_429_retry_enabled
            .or(Some(existing.upstream_429_retry_enabled != 0)),
        payload
            .upstream_429_max_retries
            .or(Some(decode_group_upstream_429_max_retries(
                existing.upstream_429_max_retries,
            ))),
        Some(match payload.available_models {
            OptionalField::Missing => {
                parse_string_array_json(existing.available_models_json.as_deref())
            }
            OptionalField::Null => Vec::new(),
            OptionalField::Value(value) => value,
        }),
    )?;
    let detail = persist_tag_update(&state.pool, id, &name, &rule)
        .await
        .map_err(map_tag_write_error)?;
    if let Err(err) =
        publish_account_effective_routing_rules_changed(state.as_ref(), None, &[]).await
    {
        warn!(
            ?err,
            tag_id = id,
            "tag update committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
    Ok(Json(detail))
}

pub(crate) async fn delete_tag(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    state.upstream_accounts.require_crypto_key()?;
    delete_tag_by_id(&state.pool, id).await?;
    if let Err(err) =
        publish_account_effective_routing_rules_changed(state.as_ref(), None, &[]).await
    {
        warn!(
            ?err,
            tag_id = id,
            "tag delete committed but routing publication failed"
        );
        invalidate_dashboard_activity_snapshots_with_accounts(
            state.dashboard_activity_snapshot_cache.as_ref(),
            "account_effective_routing_rules_publication_failed",
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(crate) async fn delete_upstream_account_group(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(group_name): AxumPath<String>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    let account_count = group_account_count_conn(tx.as_mut(), &group_name)
        .await
        .map_err(internal_error_tuple)?;
    if account_count > 0 {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "group still has {account_count} account{}; move them out before deleting",
                if account_count == 1 { "" } else { "s" }
            ),
        ));
    }
    let deleted = sqlx::query(
        r#"
        DELETE FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind(&group_name)
    .execute(tx.as_mut())
    .await
    .map_err(internal_error_tuple)?
    .rows_affected();
    if deleted == 0 {
        return Err((StatusCode::NOT_FOUND, "group not found".to_string()));
    }
    tx.commit().await.map_err(internal_error_tuple)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn get_upstream_account(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
    Query(params): Query<GetUpstreamAccountQuery>,
) -> Result<Json<UpstreamAccountDetail>, (StatusCode, String)> {
    let detail = load_upstream_account_detail_with_actual_usage_options(
        state.as_ref(),
        id,
        LoadUpstreamAccountDetailOptions {
            include_recent_actions: params.include_recent_actions.unwrap_or(false),
        },
    )
    .await
    .map_err(internal_error_tuple)?
    .ok_or_else(|| (StatusCode::NOT_FOUND, "account not found".to_string()))?;
    Ok(Json(detail))
}

pub(crate) async fn get_upstream_account_model_routing(
    State(state): State<Arc<AppState>>,
    AxumPath(id): AxumPath<i64>,
) -> Result<Json<Vec<ModelRoutingState>>, (StatusCode, String)> {
    let Some(detail) = load_upstream_account_detail_with_actual_usage_options(
        state.as_ref(),
        id,
        LoadUpstreamAccountDetailOptions {
            include_recent_actions: false,
        },
    )
    .await
    .map_err(internal_error_tuple)?
    else {
        return Err((StatusCode::NOT_FOUND, "account not found".to_string()));
    };
    Ok(Json(detail.model_routing_states))
}

pub(crate) async fn reset_upstream_account_model_routing(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    AxumPath(id): AxumPath<i64>,
    Json(payload): Json<ResetModelRoutingRequest>,
) -> Result<Json<ModelRoutingState>, (StatusCode, String)> {
    if !is_same_origin_settings_write(&headers) {
        return Err((
            StatusCode::FORBIDDEN,
            "cross-origin account writes are forbidden".to_string(),
        ));
    }
    let Some(model_state) = reset_model_route(&state.pool, id, &payload.model)
        .await
        .map_err(internal_error_tuple)?
    else {
        return Err((
            StatusCode::NOT_FOUND,
            "API Key model route not found".to_string(),
        ));
    };
    state
        .subscription_hub
        .publish_runtime_mutation(RuntimeMutation::ModelRoutingChanged);
    publish_pool_routing_availability(state.as_ref());
    Ok(Json(model_state))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GetUpstreamAccountQuery {
    pub(crate) include_recent_actions: Option<bool>,
}

#[cfg(test)]
mod model_routing_live_api_tests {
    use super::*;

    #[test]
    fn model_routing_live_api_defaults_validate_and_bound_the_query() {
        let (minutes, model, state, limit) =
            normalize_model_routing_live_query(&ModelRoutingLiveQuery::default())
                .expect("default query is valid");
        assert_eq!(minutes, 60);
        assert_eq!(model, None);
        assert_eq!(state, None);
        assert_eq!(limit, MODEL_ROUTING_LIVE_DEFAULT_LIMIT);

        let (minutes, model, state, limit) =
            normalize_model_routing_live_query(&ModelRoutingLiveQuery {
                window: Some("24h".to_string()),
                model: Some(" gpt-5.5 ".to_string()),
                state: Some(MODEL_ROUTE_STATE_COOLING_DOWN.to_string()),
                limit: Some(999),
            })
            .expect("filtered query is valid");
        assert_eq!(minutes, 1_440);
        assert_eq!(model.as_deref(), Some("gpt-5.5"));
        assert_eq!(state.as_deref(), Some(MODEL_ROUTE_STATE_COOLING_DOWN));
        assert_eq!(limit, MODEL_ROUTING_LIVE_DEFAULT_LIMIT);

        assert!(
            normalize_model_routing_live_query(&ModelRoutingLiveQuery {
                window: Some("48h".to_string()),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            normalize_model_routing_live_query(&ModelRoutingLiveQuery {
                state: Some("unknown".to_string()),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn model_routing_history_cursor_is_stable_and_rejects_invalid_values() {
        let cursor = ModelRoutingHistoryCursor {
            occurred_epoch_ms: 1_723_777_600_123,
            kind_rank: 1,
            id: 42,
        };
        let encoded = encode_model_routing_history_cursor(&cursor);
        let decoded = decode_model_routing_history_cursor(&encoded).expect("cursor round trip");
        assert_eq!(decoded.occurred_epoch_ms, cursor.occurred_epoch_ms);
        assert_eq!(decoded.kind_rank, cursor.kind_rank);
        assert_eq!(decoded.id, cursor.id);
        assert!(decode_model_routing_history_cursor("not-a-cursor").is_err());
    }

    #[test]
    fn model_routing_total_latency_requires_valid_stream_evidence() {
        assert_eq!(
            model_routing_total_latency_ms(Some(1.0), Some(2.0), Some(3.0)),
            Some(6.0)
        );
        assert_eq!(
            model_routing_total_latency_ms(Some(1.0), Some(2.0), Some(0.0)),
            None
        );
        assert_eq!(
            model_routing_total_latency_ms(Some(-1.0), Some(2.0), Some(3.0)),
            None
        );
        assert_eq!(
            model_routing_total_latency_ms(Some(f64::INFINITY), Some(2.0), Some(3.0)),
            None
        );
    }

    #[test]
    fn model_routing_display_names_use_current_api_key_names_for_records_and_audits() {
        let mut entries = vec![ModelRoutingTimelineEntry {
            occurred_epoch_ms: 1,
            kind_rank: 1,
            id: 1,
            record: ModelRoutingTimelineRecord {
                id: "attempt:1".to_string(),
                kind: "attempt".to_string(),
                occurred_at: "2026-08-23T10:00:00Z".to_string(),
                account_id: 11,
                account_display_name: None,
                model: "gpt-5.5".to_string(),
                attempt_id: None,
                invoke_id: None,
                attempt_index: None,
                same_account_retry_index: None,
                routing_source: None,
                routing_selection_audit: Some(PoolRoutingSelectionAudit {
                    selected_account_id: 11,
                    selected_account_name: "stale-selected-name".to_string(),
                    eligible_candidate_count: 2,
                    winner_reason_code: "lowest_effective_load".to_string(),
                    compared_account_id: Some(12),
                    compared_account_name: Some("stale-compared-name".to_string()),
                    selected_score: None,
                    compared_score: None,
                    handoff_admission: None,
                    excluded_candidates: vec![PoolRoutingSelectionAuditExcludedCandidate {
                        account_id: 13,
                        account_name: "stale-excluded-name".to_string(),
                        reason_code: "cooling_down".to_string(),
                    }],
                }),
                status: None,
                http_status: None,
                failure_kind: None,
                total_latency_ms: None,
                action: None,
                source: None,
                reason_code: None,
                model_route_state_before: None,
                model_route_state_after: None,
                model_route_priority_before: None,
                model_route_priority_after: None,
                model_route_failure_count: None,
                model_route_cooldown_until: None,
            },
        }];
        let display_names = std::collections::BTreeMap::from([
            (11, "API Key #11".to_string()),
            (12, "API Key #12".to_string()),
        ]);

        apply_model_routing_display_names(&mut entries, &display_names);

        let record = &entries[0].record;
        assert_eq!(record.account_display_name.as_deref(), Some("API Key #11"));
        let audit = record
            .routing_selection_audit
            .as_ref()
            .expect("selection audit is retained");
        assert_eq!(audit.selected_account_name, "API Key #11");
        assert_eq!(audit.compared_account_name.as_deref(), Some("API Key #12"));
        assert_eq!(audit.excluded_candidates[0].account_name, "API Key #13");
    }
}
