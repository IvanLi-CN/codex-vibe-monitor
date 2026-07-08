use super::*;
use anyhow::anyhow;
use chrono::LocalResult;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use tokio::sync::{broadcast, watch};
use tracing::{debug, warn};

pub(crate) struct PromptCacheConversationHydrationSnapshot<'a> {
    pub(crate) snapshot_upper_bound: &'a str,
    pub(crate) snapshot_created_at_upper_bound: Option<&'a str>,
    pub(crate) snapshot_hour_start_epoch: i64,
    pub(crate) snapshot_hour_start_bound: &'a str,
    pub(crate) snapshot_boundary_row_id_ceiling: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct PromptCacheConversationSnapshotFilter {
    pub(crate) snapshot_upper_bound: String,
    pub(crate) snapshot_created_at_upper_bound: Option<String>,
    pub(crate) snapshot_boundary_row_id_ceiling: Option<i64>,
}

impl PromptCacheConversationSnapshotFilter {
    pub(crate) fn snapshot_upper_bound(&self) -> &str {
        self.snapshot_upper_bound.as_str()
    }

    pub(crate) fn snapshot_created_at_upper_bound(&self) -> Option<&str> {
        self.snapshot_created_at_upper_bound.as_deref()
    }
}

pub(crate) fn push_snapshot_invocation_visibility_clause(
    query: &mut QueryBuilder<Sqlite>,
    occurred_at_expr: &str,
    id_expr: &str,
    created_at_expr: &str,
    snapshot: Option<&PromptCacheConversationSnapshotFilter>,
) {
    if let Some(snapshot) = snapshot {
        let snapshot_upper_bound = snapshot.snapshot_upper_bound().to_string();
        query.push("(");
        if let Some(created_at_upper_bound) = snapshot.snapshot_created_at_upper_bound() {
            query
                .push("julianday(")
                .push(created_at_expr)
                .push(") <= julianday(")
                .push_bind(created_at_upper_bound.to_string())
                .push(") AND ");
        }
        if let Some(row_id_ceiling) = snapshot.snapshot_boundary_row_id_ceiling {
            let boundary_occurred_at = parse_to_utc_datetime(&snapshot_upper_bound)
                .map(|upper_bound| {
                    db_occurred_at_lower_bound(upper_bound - ChronoDuration::seconds(1))
                })
                .unwrap_or_else(|| snapshot_upper_bound.clone());
            query
                .push("((")
                .push(occurred_at_expr)
                .push(" < ")
                .push_bind(boundary_occurred_at.clone())
                .push(") OR (")
                .push(occurred_at_expr)
                .push(" = ")
                .push_bind(boundary_occurred_at)
                .push(" AND ")
                .push(id_expr)
                .push(" <= ")
                .push_bind(row_id_ceiling)
                .push("))");
        } else {
            query
                .push("(")
                .push(occurred_at_expr)
                .push(" < ")
                .push_bind(snapshot_upper_bound)
                .push(")");
        }
        query.push(")");
    }
}

pub(crate) async fn hydrate_prompt_cache_conversations(
    state: &AppState,
    source_scope: InvocationSourceScope,
    aggregates: Vec<PromptCacheConversationAggregateRow>,
    range_end: DateTime<Utc>,
    detail_level: PromptCacheConversationDetailLevel,
    recent_invocation_limit: Option<i64>,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
    runtime_overlay_records: &[ApiInvocation],
) -> Result<Vec<PromptCacheConversationResponse>> {
    if aggregates.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let recent_invocation_limit = match detail_level {
        PromptCacheConversationDetailLevel::Full => recent_invocation_limit
            .unwrap_or(PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT as i64),
        PromptCacheConversationDetailLevel::Compact => recent_invocation_limit.unwrap_or(2),
    };

    let events = if detail_level == PromptCacheConversationDetailLevel::Full {
        let chart_range_start_bound = resolve_prompt_cache_conversation_chart_range_start(
            range_end,
            aggregates.iter().map(|row| row.created_at.as_str()).min(),
        );
        query_prompt_cache_conversation_events(
            &state.pool,
            &chart_range_start_bound,
            snapshot,
            source_scope,
            &selected_keys,
        )
        .await?
    } else {
        Vec::new()
    };

    let upstream_account_rows = if detail_level == PromptCacheConversationDetailLevel::Full {
        if let Some(snapshot) = snapshot {
            query_prompt_cache_conversation_upstream_account_summaries_at_snapshot(
                &state.pool,
                source_scope,
                &selected_keys,
                snapshot.snapshot_hour_start_epoch,
                snapshot.snapshot_hour_start_bound,
                snapshot,
            )
            .await?
        } else {
            query_prompt_cache_conversation_upstream_account_summaries(
                &state.pool,
                source_scope,
                &selected_keys,
            )
            .await?
        }
    } else {
        Vec::new()
    };

    let recent_invocation_rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        source_scope,
        &selected_keys,
        recent_invocation_limit,
        snapshot,
    )
    .await?;
    let encrypted_owner_rows = if state
        .proxy_model_settings
        .read()
        .await
        .encrypted_session_owner_routing_enabled
    {
        if let Some(snapshot) = snapshot {
            query_prompt_cache_conversation_encrypted_owner_summaries_at_snapshot(
                &state.pool,
                source_scope,
                &selected_keys,
                snapshot,
            )
            .await?
        } else {
            query_prompt_cache_conversation_encrypted_owner_summaries(&state.pool, &selected_keys)
                .await?
        }
    } else {
        Vec::new()
    };

    let mut grouped_events: HashMap<String, Vec<PromptCacheConversationRequestPointResponse>> =
        HashMap::new();
    for row in events {
        let normalized_status = row.status.trim().to_string();
        let display_status = if normalized_status.is_empty() {
            "unknown".to_string()
        } else {
            normalized_status.clone()
        };
        let outcome = invocation_point_outcome(
            Some(&normalized_status),
            row.error_message.as_deref(),
            row.downstream_error_message.as_deref(),
            row.failure_kind.as_deref(),
            row.failure_class.as_deref(),
        )
        .to_string();
        let request_tokens = row.request_tokens.max(0);
        let points = grouped_events.entry(row.prompt_cache_key).or_default();
        let cumulative_tokens = points
            .last()
            .map(|point| point.cumulative_tokens)
            .unwrap_or(0)
            + request_tokens;
        points.push(PromptCacheConversationRequestPointResponse {
            occurred_at: row.occurred_at,
            status: display_status,
            is_success: outcome == "success",
            outcome,
            request_tokens,
            cumulative_tokens,
        });
    }

    let mut upstream_account_rows_by_key: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
    > = HashMap::new();
    for row in upstream_account_rows {
        upstream_account_rows_by_key
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(row);
    }
    let mut grouped_recent_invocations: HashMap<
        String,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    > = HashMap::new();
    for row in recent_invocation_rows {
        grouped_recent_invocations
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(prompt_cache_invocation_preview_from_row(row));
    }
    overlay_runtime_prompt_cache_invocation_previews(
        &mut grouped_recent_invocations,
        runtime_overlay_records,
        &selected_keys,
        recent_invocation_limit,
    );

    let mut grouped_upstream_accounts: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountResponse>,
    > = HashMap::new();
    for (prompt_cache_key, rows) in upstream_account_rows_by_key {
        let mut unique_ids_by_name: HashMap<String, Option<i64>> = HashMap::new();
        for row in &rows {
            let Some(normalized_name) =
                normalize_trimmed_optional_string(row.upstream_account_name.clone())
            else {
                continue;
            };
            let Some(upstream_account_id) = row.upstream_account_id else {
                continue;
            };
            match unique_ids_by_name.entry(normalized_name) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(upstream_account_id));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    if entry
                        .get()
                        .is_some_and(|existing_id| existing_id != upstream_account_id)
                    {
                        entry.insert(None);
                    }
                }
            }
        }

        let mut account_entries: HashMap<String, PromptCacheConversationUpstreamAccountResponse> =
            HashMap::new();
        for row in rows {
            let normalized_name =
                normalize_trimmed_optional_string(row.upstream_account_name.clone());
            let resolved_upstream_account_id = row.upstream_account_id.or_else(|| {
                normalized_name
                    .as_ref()
                    .and_then(|name| unique_ids_by_name.get(name).copied().flatten())
            });
            let account_group_key = resolve_prompt_cache_upstream_account_group_key(
                resolved_upstream_account_id,
                normalized_name.as_deref(),
            );
            let entry = account_entries.entry(account_group_key).or_insert_with(|| {
                PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: resolved_upstream_account_id,
                    upstream_account_name: normalized_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: row.last_activity_at.clone(),
                }
            });

            if entry.upstream_account_id.is_none() && resolved_upstream_account_id.is_some() {
                entry.upstream_account_id = resolved_upstream_account_id;
            }
            if entry.upstream_account_name.is_none() && normalized_name.is_some() {
                entry.upstream_account_name = normalized_name;
            }
            entry.request_count += row.request_count;
            entry.total_tokens += row.total_tokens.max(0);
            entry.total_cost += row.total_cost;
            if row.last_activity_at > entry.last_activity_at {
                entry.last_activity_at = row.last_activity_at;
            }
        }
        grouped_upstream_accounts.insert(
            prompt_cache_key,
            account_entries.into_values().collect::<Vec<_>>(),
        );
    }

    for accounts in grouped_upstream_accounts.values_mut() {
        accounts.sort_by(|left, right| {
            right
                .last_activity_at
                .cmp(&left.last_activity_at)
                .then_with(|| {
                    resolve_prompt_cache_upstream_account_label(
                        right.upstream_account_name.as_deref(),
                        right.upstream_account_id,
                    )
                    .cmp(&resolve_prompt_cache_upstream_account_label(
                        left.upstream_account_name.as_deref(),
                        left.upstream_account_id,
                    ))
                })
                .then_with(|| {
                    right
                        .upstream_account_id
                        .unwrap_or(i64::MIN)
                        .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
                })
                .then_with(|| right.total_tokens.cmp(&left.total_tokens))
                .then_with(|| right.request_count.cmp(&left.request_count))
        });
        accounts.truncate(PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT);
    }

    let mut encrypted_owner_rows_by_key: HashMap<
        String,
        PromptCacheConversationEncryptedOwnerSummaryRow,
    > = encrypted_owner_rows
        .into_iter()
        .map(|row| (row.prompt_cache_key.clone(), row))
        .collect();

    let conversations = aggregates
        .into_iter()
        .map(|row| {
            let owner = encrypted_owner_rows_by_key.remove(&row.prompt_cache_key);
            PromptCacheConversationResponse {
                prompt_cache_key: row.prompt_cache_key.clone(),
                request_count: row.request_count,
                total_tokens: row.total_tokens,
                total_cost: row.total_cost,
                created_at: row.created_at,
                last_activity_at: row.last_activity_at,
                last_terminal_at: row.last_terminal_at,
                last_in_flight_at: row.last_in_flight_at,
                cursor: None,
                has_encrypted_session_owner: owner.is_some(),
                encrypted_owner_account_id: owner
                    .as_ref()
                    .map(|value| value.owner_upstream_account_id),
                encrypted_owner_account_name: owner
                    .as_ref()
                    .and_then(|value| value.owner_upstream_account_name.clone()),
                encrypted_owner_group_name: owner
                    .as_ref()
                    .and_then(|value| value.owner_group_name.clone()),
                upstream_accounts: grouped_upstream_accounts
                    .remove(&row.prompt_cache_key)
                    .unwrap_or_default(),
                recent_invocations: grouped_recent_invocations
                    .remove(&row.prompt_cache_key)
                    .unwrap_or_default(),
                last24h_requests: grouped_events
                    .remove(&row.prompt_cache_key)
                    .unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();

    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    if elapsed_ms >= 250 {
        tracing::warn!(
            endpoint = "/api/prompt-cache/conversations",
            window = if snapshot.is_some() {
                "snapshot"
            } else {
                "live"
            },
            ?source_scope,
            selected_key_count = selected_keys.len() as i64,
            row_count = conversations.len() as i64,
            cache_hit_or_miss = "hydrate",
            elapsed_ms,
            "prompt cache conversation hydration exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            endpoint = "/api/prompt-cache/conversations",
            window = if snapshot.is_some() {
                "snapshot"
            } else {
                "live"
            },
            ?source_scope,
            selected_key_count = selected_keys.len() as i64,
            row_count = conversations.len() as i64,
            cache_hit_or_miss = "hydrate",
            elapsed_ms,
            "prompt cache conversation hydration completed"
        );
    }

    Ok(conversations)
}

pub(crate) fn overlay_runtime_prompt_cache_invocation_previews(
    grouped_recent_invocations: &mut HashMap<
        String,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    >,
    runtime_overlay_records: &[ApiInvocation],
    selected_keys: &[String],
    recent_invocation_limit: i64,
) {
    if runtime_overlay_records.is_empty()
        || selected_keys.is_empty()
        || recent_invocation_limit <= 0
    {
        return;
    }
    let selected_keys = selected_keys.iter().collect::<HashSet<_>>();
    for record in runtime_overlay_records {
        let Some(prompt_cache_key) =
            normalize_trimmed_optional_string(record.prompt_cache_key.clone())
        else {
            continue;
        };
        if !selected_keys.contains(&prompt_cache_key) {
            continue;
        }
        let previews = grouped_recent_invocations
            .entry(prompt_cache_key.clone())
            .or_default();
        if previews.iter().any(|preview| {
            preview.invoke_id == record.invoke_id && preview.occurred_at == record.occurred_at
        }) {
            continue;
        }
        previews.push(prompt_cache_invocation_preview_from_runtime_record(
            record,
            prompt_cache_key,
        ));
    }

    for previews in grouped_recent_invocations.values_mut() {
        previews.sort_by(|left, right| {
            right
                .occurred_at
                .cmp(&left.occurred_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        previews.truncate(recent_invocation_limit as usize);
    }
}

pub(crate) fn prompt_cache_invocation_preview_from_runtime_record(
    record: &ApiInvocation,
    prompt_cache_key: String,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: record.id,
        invoke_id: record.invoke_id.clone(),
        prompt_cache_key: Some(prompt_cache_key),
        occurred_at: record.occurred_at.clone(),
        status: record
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        live_phase: record
            .live_phase
            .clone()
            .or_else(|| runtime_invocation_live_phase(&record).map(str::to_string)),
        failure_class: normalize_trimmed_optional_string(record.failure_class.clone()),
        route_mode: normalize_trimmed_optional_string(record.route_mode.clone()),
        model: normalize_trimmed_optional_string(record.model.clone()),
        request_model: normalize_trimmed_optional_string(record.request_model.clone()),
        response_model: normalize_trimmed_optional_string(record.response_model.clone()),
        total_tokens: record.total_tokens.unwrap_or_default().max(0),
        cost: record.cost,
        proxy_display_name: normalize_trimmed_optional_string(record.proxy_display_name.clone()),
        upstream_account_id: record.upstream_account_id,
        upstream_account_name: normalize_trimmed_optional_string(
            record.upstream_account_name.clone(),
        ),
        upstream_account_plan_type: None,
        endpoint: normalize_trimmed_optional_string(record.endpoint.clone()),
        compaction_request_kind: normalize_trimmed_optional_string(
            record.compaction_request_kind.clone(),
        ),
        compaction_response_kind: normalize_trimmed_optional_string(
            record.compaction_response_kind.clone(),
        ),
        image_intent: normalize_trimmed_optional_string(record.image_intent.clone()),
        source: normalize_trimmed_optional_string(Some(record.source.clone())),
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cache_input_tokens: record.cache_input_tokens,
        reasoning_tokens: record.reasoning_tokens,
        reasoning_effort: normalize_trimmed_optional_string(record.reasoning_effort.clone()),
        error_message: normalize_trimmed_optional_string(record.error_message.clone()),
        downstream_status_code: record.downstream_status_code,
        downstream_error_message: normalize_trimmed_optional_string(
            record.downstream_error_message.clone(),
        ),
        failure_kind: normalize_trimmed_optional_string(record.failure_kind.clone()),
        is_actionable: record.is_actionable,
        response_content_encoding: normalize_trimmed_optional_string(
            record.response_content_encoding.clone(),
        ),
        transport: normalize_trimmed_optional_string(record.transport.clone()),
        requested_service_tier: normalize_trimmed_optional_string(
            record.requested_service_tier.clone(),
        ),
        service_tier: normalize_trimmed_optional_string(record.service_tier.clone()),
        billing_service_tier: normalize_trimmed_optional_string(
            record.billing_service_tier.clone(),
        ),
        t_req_read_ms: record.t_req_read_ms,
        t_req_parse_ms: record.t_req_parse_ms,
        t_upstream_connect_ms: record.t_upstream_connect_ms,
        t_upstream_ttfb_ms: record.t_upstream_ttfb_ms,
        t_upstream_stream_ms: record.t_upstream_stream_ms,
        t_resp_parse_ms: record.t_resp_parse_ms,
        t_persist_ms: record.t_persist_ms,
        t_total_ms: record.t_total_ms,
    }
}

pub(crate) fn resolve_prompt_cache_conversation_chart_range_start(
    range_end: DateTime<Utc>,
    earliest_created_at: Option<&str>,
) -> String {
    let floor = range_end - ChronoDuration::hours(PROMPT_CACHE_CONVERSATION_CHART_MAX_HOURS);
    let created_at = earliest_created_at
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc));
    let chart_start = match created_at {
        Some(created_at) if created_at > floor => created_at,
        _ => floor,
    };
    format_utc_iso(chart_start)
}

pub(crate) fn normalize_trimmed_optional_string(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn prompt_cache_invocation_preview_from_row(
    row: PromptCacheConversationInvocationPreviewRow,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: row.id,
        invoke_id: row.invoke_id,
        prompt_cache_key: Some(row.prompt_cache_key),
        occurred_at: row.occurred_at,
        status: row.status,
        live_phase: normalize_trimmed_optional_string(row.live_phase),
        failure_class: normalize_trimmed_optional_string(row.failure_class),
        route_mode: normalize_trimmed_optional_string(row.route_mode),
        model: normalize_trimmed_optional_string(row.model),
        request_model: normalize_trimmed_optional_string(row.request_model),
        response_model: normalize_trimmed_optional_string(row.response_model),
        total_tokens: row.total_tokens.max(0),
        cost: row.cost,
        proxy_display_name: normalize_trimmed_optional_string(row.proxy_display_name),
        upstream_account_id: row.upstream_account_id,
        upstream_account_name: normalize_trimmed_optional_string(row.upstream_account_name),
        upstream_account_plan_type: normalize_trimmed_optional_string(
            row.upstream_account_plan_type,
        ),
        endpoint: normalize_trimmed_optional_string(row.endpoint),
        compaction_request_kind: normalize_trimmed_optional_string(row.compaction_request_kind),
        compaction_response_kind: normalize_trimmed_optional_string(row.compaction_response_kind),
        image_intent: normalize_trimmed_optional_string(row.image_intent),
        source: normalize_trimmed_optional_string(row.source),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_input_tokens: row.cache_input_tokens,
        reasoning_tokens: row.reasoning_tokens,
        reasoning_effort: normalize_trimmed_optional_string(row.reasoning_effort),
        error_message: normalize_trimmed_optional_string(row.error_message),
        downstream_status_code: row.downstream_status_code,
        downstream_error_message: normalize_trimmed_optional_string(row.downstream_error_message),
        failure_kind: normalize_trimmed_optional_string(row.failure_kind),
        is_actionable: row.is_actionable.map(|value| value != 0),
        response_content_encoding: normalize_trimmed_optional_string(row.response_content_encoding),
        transport: normalize_trimmed_optional_string(row.transport),
        requested_service_tier: normalize_trimmed_optional_string(row.requested_service_tier),
        service_tier: normalize_trimmed_optional_string(row.service_tier),
        billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
        t_req_read_ms: row.t_req_read_ms,
        t_req_parse_ms: row.t_req_parse_ms,
        t_upstream_connect_ms: row.t_upstream_connect_ms,
        t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
        t_upstream_stream_ms: row.t_upstream_stream_ms,
        t_resp_parse_ms: row.t_resp_parse_ms,
        t_persist_ms: row.t_persist_ms,
        t_total_ms: row.t_total_ms,
    }
}

pub(crate) fn upstream_account_invocation_preview_from_row(
    row: UpstreamAccountInvocationPreviewRow,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: row.id,
        invoke_id: row.invoke_id,
        prompt_cache_key: row.prompt_cache_key,
        occurred_at: row.occurred_at,
        status: row.status,
        live_phase: normalize_trimmed_optional_string(row.live_phase),
        failure_class: normalize_trimmed_optional_string(row.failure_class),
        route_mode: normalize_trimmed_optional_string(row.route_mode),
        model: normalize_trimmed_optional_string(row.model),
        request_model: normalize_trimmed_optional_string(row.request_model),
        response_model: normalize_trimmed_optional_string(row.response_model),
        total_tokens: row.total_tokens.max(0),
        cost: row.cost,
        proxy_display_name: normalize_trimmed_optional_string(row.proxy_display_name),
        upstream_account_id: row.upstream_account_id,
        upstream_account_name: normalize_trimmed_optional_string(row.upstream_account_name),
        upstream_account_plan_type: normalize_trimmed_optional_string(
            row.upstream_account_plan_type,
        ),
        endpoint: normalize_trimmed_optional_string(row.endpoint),
        compaction_request_kind: normalize_trimmed_optional_string(row.compaction_request_kind),
        compaction_response_kind: normalize_trimmed_optional_string(row.compaction_response_kind),
        image_intent: normalize_trimmed_optional_string(row.image_intent),
        source: normalize_trimmed_optional_string(row.source),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_input_tokens: row.cache_input_tokens,
        reasoning_tokens: row.reasoning_tokens,
        reasoning_effort: normalize_trimmed_optional_string(row.reasoning_effort),
        error_message: normalize_trimmed_optional_string(row.error_message),
        downstream_status_code: row.downstream_status_code,
        downstream_error_message: normalize_trimmed_optional_string(row.downstream_error_message),
        failure_kind: normalize_trimmed_optional_string(row.failure_kind),
        is_actionable: row.is_actionable.map(|value| value != 0),
        response_content_encoding: normalize_trimmed_optional_string(row.response_content_encoding),
        transport: normalize_trimmed_optional_string(row.transport),
        requested_service_tier: normalize_trimmed_optional_string(row.requested_service_tier),
        service_tier: normalize_trimmed_optional_string(row.service_tier),
        billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
        t_req_read_ms: row.t_req_read_ms,
        t_req_parse_ms: row.t_req_parse_ms,
        t_upstream_connect_ms: row.t_upstream_connect_ms,
        t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
        t_upstream_stream_ms: row.t_upstream_stream_ms,
        t_resp_parse_ms: row.t_resp_parse_ms,
        t_persist_ms: row.t_persist_ms,
        t_total_ms: row.t_total_ms,
    }
}

pub(crate) fn resolve_prompt_cache_upstream_account_label(
    upstream_account_name: Option<&str>,
    upstream_account_id: Option<i64>,
) -> String {
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return name.to_string();
    }
    if let Some(account_id) = upstream_account_id {
        return format!("账号 #{account_id}");
    }
    "—".to_string()
}

pub(crate) fn resolve_prompt_cache_upstream_account_group_key(
    upstream_account_id: Option<i64>,
    upstream_account_name: Option<&str>,
) -> String {
    if let Some(account_id) = upstream_account_id {
        return format!("id:{account_id}");
    }
    if let Some(name) = upstream_account_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return format!("name:{name}");
    }
    "unknown".to_string()
}
