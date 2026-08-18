use super::*;

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
    let mut connection = state.pool.acquire().await?;
    hydrate_prompt_cache_conversations_on_connection(
        state,
        &mut connection,
        source_scope,
        aggregates,
        range_end,
        detail_level,
        recent_invocation_limit,
        snapshot,
        runtime_overlay_records,
    )
    .await
}

pub(crate) async fn hydrate_prompt_cache_conversations_on_connection(
    state: &AppState,
    connection: &mut SqliteConnection,
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

    let chart_range_start_bound =
        (detail_level == PromptCacheConversationDetailLevel::Full).then(|| {
            resolve_prompt_cache_conversation_chart_range_start(
                range_end,
                aggregates.iter().map(|row| row.created_at.as_str()).min(),
            )
        });
    let mut events = if let Some(chart_range_start_bound) = chart_range_start_bound.as_deref() {
        query_prompt_cache_conversation_events(
            &mut *connection,
            chart_range_start_bound,
            snapshot,
            source_scope,
            &selected_keys,
        )
        .await?
    } else {
        Vec::new()
    };

    let mut upstream_account_rows = if detail_level == PromptCacheConversationDetailLevel::Full {
        if let Some(snapshot) = snapshot {
            query_prompt_cache_conversation_upstream_account_summaries_at_snapshot(
                &mut *connection,
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
        &mut *connection,
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
                &mut *connection,
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
    let manual_binding_rows =
        query_prompt_cache_conversation_manual_binding_summaries(&mut *connection, &selected_keys)
            .await?;

    // Runtime records use id 0 until persistence assigns their SQLite identity. The aggregate
    // overlay already selects them, so detail hydration must contribute the same transient rows
    // to charts and account summaries instead of publishing totals without their wire details.
    if detail_level == PromptCacheConversationDetailLevel::Full {
        let selected_keys = selected_keys
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        for record in runtime_overlay_records
            .iter()
            .filter(|record| record.id <= 0)
        {
            let Some(prompt_cache_key) = record
                .prompt_cache_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
            else {
                continue;
            };
            if !selected_keys.contains(prompt_cache_key) {
                continue;
            }
            let status = record
                .status
                .as_deref()
                .map(str::trim)
                .filter(|status| !status.is_empty())
                .unwrap_or("unknown")
                .to_string();
            let is_within_chart_range =
                chart_range_start_bound
                    .as_deref()
                    .is_some_and(|chart_start| {
                        parse_to_utc_datetime(chart_start).is_some_and(|chart_start| {
                            parse_to_utc_datetime(&record.occurred_at)
                                .is_some_and(|occurred_at| occurred_at >= chart_start)
                        })
                    });
            if is_within_chart_range {
                events.push(PromptCacheConversationEventRow {
                    occurred_at: record.occurred_at.clone(),
                    status,
                    error_message: record.error_message.clone(),
                    downstream_error_message: record.downstream_error_message.clone(),
                    failure_kind: record.failure_kind.clone(),
                    failure_class: record.failure_class.clone(),
                    request_tokens: record.total_tokens.unwrap_or_default().max(0),
                    prompt_cache_key: prompt_cache_key.to_string(),
                });
            }
            upstream_account_rows.push(PromptCacheConversationUpstreamAccountSummaryRow {
                prompt_cache_key: prompt_cache_key.to_string(),
                upstream_account_id: record.upstream_account_id,
                upstream_account_name: normalize_trimmed_optional_string(
                    record.upstream_account_name.clone(),
                ),
                request_count: 1,
                total_tokens: record.total_tokens.unwrap_or_default().max(0),
                total_cost: record.cost.unwrap_or_default(),
                last_activity_at: record.occurred_at.clone(),
            });
        }
    }

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
    for points in grouped_events.values_mut() {
        points.sort_by(|left, right| left.occurred_at.cmp(&right.occurred_at));
        let mut cumulative_tokens = 0_i64;
        for point in points {
            cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
            point.cumulative_tokens = cumulative_tokens;
        }
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
    let mut manual_binding_by_key: HashMap<String, PromptCacheConversationManualBindingResponse> =
        manual_binding_rows
            .into_iter()
            .filter_map(prompt_cache_manual_binding_response_from_row)
            .map(|binding| {
                let prompt_cache_key = binding.prompt_cache_key.clone();
                (prompt_cache_key, binding.response)
            })
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
                manual_binding: manual_binding_by_key.remove(&row.prompt_cache_key),
                blocked_binding: grouped_recent_invocations
                    .get(&row.prompt_cache_key)
                    .and_then(|previews| {
                        previews
                            .iter()
                            .find_map(|preview| preview.blocked_binding.clone())
                    }),
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

struct PromptCacheManualBindingResponseByKey {
    prompt_cache_key: String,
    response: PromptCacheConversationManualBindingResponse,
}

fn prompt_cache_manual_binding_response_from_row(
    row: PromptCacheConversationManualBindingSummaryRow,
) -> Option<PromptCacheManualBindingResponseByKey> {
    let prompt_cache_key = row.prompt_cache_key.trim().to_string();
    if prompt_cache_key.is_empty() {
        return None;
    }

    match row.binding_kind.as_str() {
        PROMPT_CACHE_BINDING_KIND_GROUP => {
            let group_name = normalize_trimmed_optional_string(row.group_name)?;
            Some(PromptCacheManualBindingResponseByKey {
                prompt_cache_key,
                response: PromptCacheConversationManualBindingResponse {
                    binding_kind: "group".to_string(),
                    group_name: Some(group_name),
                    upstream_account_id: None,
                    upstream_account_name: None,
                },
            })
        }
        PROMPT_CACHE_BINDING_KIND_UPSTREAM_ACCOUNT => {
            let upstream_account_name =
                normalize_trimmed_optional_string(row.upstream_account_name.clone());
            let upstream_account_id = row.upstream_account_id;
            if upstream_account_name.is_none() && upstream_account_id.is_none() {
                return None;
            }
            Some(PromptCacheManualBindingResponseByKey {
                prompt_cache_key,
                response: PromptCacheConversationManualBindingResponse {
                    binding_kind: "upstreamAccount".to_string(),
                    group_name: None,
                    upstream_account_id,
                    upstream_account_name,
                },
            })
        }
        _ => None,
    }
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
        if let Some(preview) = previews.iter_mut().find(|preview| {
            preview.invoke_id == record.invoke_id && preview.occurred_at == record.occurred_at
        }) {
            overlay_runtime_preview_progress(preview, record);
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

fn overlay_runtime_preview_progress(
    preview: &mut PromptCacheConversationInvocationPreviewResponse,
    record: &ApiInvocation,
) {
    let (first_token_ms, live_phase) = merged_runtime_preview_progress(
        preview.first_token_ms,
        preview.live_phase.as_deref(),
        record.first_token_ms,
        effective_runtime_invocation_live_phase(record),
    );
    preview.first_token_ms = first_token_ms;
    preview.live_phase = live_phase;
}

fn merged_runtime_preview_progress(
    persisted_first_token_ms: Option<f64>,
    persisted_live_phase: Option<&str>,
    runtime_first_token_ms: Option<f64>,
    runtime_live_phase: Option<&str>,
) -> (Option<f64>, Option<String>) {
    let measured_runtime_first_token_ms =
        runtime_first_token_ms.filter(|value| value.is_finite() && *value >= 0.0);
    let runtime_is_responding = measured_runtime_first_token_ms.is_some()
        && runtime_live_phase == Some(INVOCATION_LIVE_PHASE_RESPONDING);

    (
        measured_runtime_first_token_ms.or(persisted_first_token_ms),
        runtime_is_responding
            .then_some(INVOCATION_LIVE_PHASE_RESPONDING.to_string())
            .or_else(|| persisted_live_phase.map(str::to_string)),
    )
}

pub(crate) fn prompt_cache_invocation_preview_from_runtime_record(
    record: &ApiInvocation,
    prompt_cache_key: String,
) -> PromptCacheConversationInvocationPreviewResponse {
    let mut preview = invocation_preview_from_runtime_record(record);
    preview.prompt_cache_key = Some(prompt_cache_key);
    preview
}

pub(crate) fn invocation_preview_from_runtime_record(
    record: &ApiInvocation,
) -> PromptCacheConversationInvocationPreviewResponse {
    PromptCacheConversationInvocationPreviewResponse {
        id: record.id,
        invoke_id: record.invoke_id.clone(),
        prompt_cache_key: record.prompt_cache_key.clone(),
        occurred_at: record.occurred_at.clone(),
        status: record
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
        live_phase: effective_runtime_invocation_live_phase(record).map(str::to_string),
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
        blocked_binding: record.blocked_binding.clone(),
        is_actionable: record.is_actionable,
        response_content_encoding: normalize_trimmed_optional_string(
            record.response_content_encoding.clone(),
        ),
        request_compression_algorithm: normalize_trimmed_optional_string(
            record.request_compression_algorithm.clone(),
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
        first_token_ms: record.first_token_ms,
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
        blocked_binding: None,
        is_actionable: row.is_actionable.map(|value| value != 0),
        response_content_encoding: normalize_trimmed_optional_string(row.response_content_encoding),
        request_compression_algorithm: normalize_trimmed_optional_string(
            row.request_compression_algorithm,
        ),
        transport: normalize_trimmed_optional_string(row.transport),
        requested_service_tier: normalize_trimmed_optional_string(row.requested_service_tier),
        service_tier: normalize_trimmed_optional_string(row.service_tier),
        billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
        t_req_read_ms: row.t_req_read_ms,
        t_req_parse_ms: row.t_req_parse_ms,
        t_upstream_connect_ms: row.t_upstream_connect_ms,
        t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
        first_token_ms: row.first_token_ms,
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
        blocked_binding: None,
        is_actionable: row.is_actionable.map(|value| value != 0),
        response_content_encoding: normalize_trimmed_optional_string(row.response_content_encoding),
        request_compression_algorithm: normalize_trimmed_optional_string(
            row.request_compression_algorithm,
        ),
        transport: normalize_trimmed_optional_string(row.transport),
        requested_service_tier: normalize_trimmed_optional_string(row.requested_service_tier),
        service_tier: normalize_trimmed_optional_string(row.service_tier),
        billing_service_tier: normalize_trimmed_optional_string(row.billing_service_tier),
        t_req_read_ms: row.t_req_read_ms,
        t_req_parse_ms: row.t_req_parse_ms,
        t_upstream_connect_ms: row.t_upstream_connect_ms,
        t_upstream_ttfb_ms: row.t_upstream_ttfb_ms,
        first_token_ms: row.first_token_ms,
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

#[cfg(test)]
mod runtime_preview_progress_tests {
    use super::*;

    #[test]
    fn runtime_first_token_progress_overlays_the_stale_persisted_preview() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
            Some(720.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
        );

        assert_eq!(first_token_ms, Some(720.0));
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn missing_runtime_timing_does_not_regress_a_persisted_responding_preview() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            Some(720.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
        );

        assert_eq!(first_token_ms, Some(720.0));
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }

    #[test]
    fn zero_millisecond_runtime_first_token_promotes_a_missing_preview() {
        let (first_token_ms, live_phase) = merged_runtime_preview_progress(
            None,
            Some(INVOCATION_LIVE_PHASE_REQUESTING),
            Some(0.0),
            Some(INVOCATION_LIVE_PHASE_RESPONDING),
        );

        assert_eq!(first_token_ms, Some(0.0));
        assert_eq!(
            live_phase.as_deref(),
            Some(INVOCATION_LIVE_PHASE_RESPONDING)
        );
    }
}
