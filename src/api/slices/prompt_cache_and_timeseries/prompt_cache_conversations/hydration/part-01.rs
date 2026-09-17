struct PromptCacheHydrationRows {
    events: Vec<PromptCacheConversationEventRow>,
    upstream_account_rows: Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
    recent_invocation_rows: Vec<PromptCacheConversationInvocationPreviewRow>,
    encrypted_owner_rows: Vec<PromptCacheConversationEncryptedOwnerSummaryRow>,
    manual_binding_rows: Vec<PromptCacheConversationManualBindingSummaryRow>,
}

async fn load_hydration_rows(
    state: &AppState,
    connection: &mut SqliteConnection,
    request: &PromptCacheConversationHydrationRequest<'_>,
    selected_keys: &[String],
    recent_invocation_limit: i64,
    chart_range_start_bound: Option<&str>,
) -> Result<PromptCacheHydrationRows> {
    let events = if let Some(chart_range_start_bound) = chart_range_start_bound {
        query_prompt_cache_conversation_events(
            &mut *connection,
            chart_range_start_bound,
            request.snapshot,
            request.source_scope,
            selected_keys,
        )
        .await?
    } else {
        Vec::new()
    };
    let upstream_account_rows = if request.detail_level == PromptCacheConversationDetailLevel::Full
    {
        if let Some(snapshot) = request.snapshot {
            query_prompt_cache_conversation_upstream_account_summaries_at_snapshot(
                &mut *connection,
                request.source_scope,
                selected_keys,
                snapshot.snapshot_hour_start_epoch,
                snapshot.snapshot_hour_start_bound,
                snapshot,
            )
            .await?
        } else {
            query_prompt_cache_conversation_upstream_account_summaries(
                &state.pool,
                request.source_scope,
                selected_keys,
            )
            .await?
        }
    } else {
        Vec::new()
    };
    let recent_invocation_rows = query_prompt_cache_conversation_recent_invocations(
        &mut *connection,
        request.source_scope,
        selected_keys,
        recent_invocation_limit,
        request.snapshot,
    )
    .await?;
    let encrypted_owner_rows = if state
        .proxy_model_settings
        .read()
        .await
        .encrypted_session_owner_routing_enabled
    {
        if let Some(snapshot) = request.snapshot {
            query_prompt_cache_conversation_encrypted_owner_summaries_at_snapshot(
                &mut *connection,
                request.source_scope,
                selected_keys,
                snapshot,
            )
            .await?
        } else {
            query_prompt_cache_conversation_encrypted_owner_summaries(&state.pool, selected_keys)
                .await?
        }
    } else {
        Vec::new()
    };
    let manual_binding_rows =
        query_prompt_cache_conversation_manual_binding_summaries(&mut *connection, selected_keys)
            .await?;
    Ok(PromptCacheHydrationRows {
        events,
        upstream_account_rows,
        recent_invocation_rows,
        encrypted_owner_rows,
        manual_binding_rows,
    })
}

async fn load_in_flight_phase_counts(
    connection: &mut SqliteConnection,
    request: &PromptCacheConversationHydrationRequest<'_>,
    selected_keys: &[String],
) -> Result<HashMap<String, InvocationPhaseCountsResponse>> {
    let mut counts = HashMap::new();
    for record in query_prompt_cache_in_flight_phase_records(
        &mut *connection,
        request.source_scope,
        selected_keys,
        request.snapshot,
    )
    .await?
    {
        counts
            .entry(record.prompt_cache_key)
            .or_insert_with(InvocationPhaseCountsResponse::default)
            .increment_phase_name(record.phase.as_deref());
    }
    for record in request.runtime_overlay_records {
        if prompt_cache_runtime_record_is_in_flight(record)
            && let Some(prompt_cache_key) = record.prompt_cache_key.as_deref()
            && selected_keys.iter().any(|key| key == prompt_cache_key)
        {
            counts
                .entry(prompt_cache_key.to_string())
                .or_insert_with(InvocationPhaseCountsResponse::default)
                .increment_phase_name(runtime_record_live_phase(record));
        }
    }
    Ok(counts)
}

fn append_transient_runtime_hydration_rows(
    request: &PromptCacheConversationHydrationRequest<'_>,
    selected_keys: &[String],
    chart_range_start_bound: Option<&str>,
    events: &mut Vec<PromptCacheConversationEventRow>,
    upstream_account_rows: &mut Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
) {
    if request.detail_level != PromptCacheConversationDetailLevel::Full {
        return;
    }
    let selected_keys = selected_keys
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    for record in request
        .runtime_overlay_records
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
        if chart_range_start_bound.is_some_and(|chart_start| {
            parse_to_utc_datetime(chart_start).is_some_and(|chart_start| {
                parse_to_utc_datetime(&record.occurred_at)
                    .is_some_and(|occurred_at| occurred_at >= chart_start)
            })
        }) {
            events.push(PromptCacheConversationEventRow {
                occurred_at: record.occurred_at.clone(),
                status: status.clone(),
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

fn group_hydration_events(
    events: Vec<PromptCacheConversationEventRow>,
) -> HashMap<String, Vec<PromptCacheConversationRequestPointResponse>> {
    let mut grouped = HashMap::new();
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
        let points = grouped.entry(row.prompt_cache_key).or_insert_with(Vec::new);
        let cumulative_tokens = points
            .last()
            .map(|point: &PromptCacheConversationRequestPointResponse| point.cumulative_tokens)
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
    for points in grouped.values_mut() {
        points.sort_by(|left, right| left.occurred_at.cmp(&right.occurred_at));
        let mut cumulative_tokens = 0_i64;
        for point in points {
            cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
            point.cumulative_tokens = cumulative_tokens;
        }
    }
    grouped
}

fn group_recent_invocations(
    rows: Vec<PromptCacheConversationInvocationPreviewRow>,
    runtime_overlay_records: &[ApiInvocation],
    selected_keys: &[String],
    recent_invocation_limit: i64,
) -> HashMap<String, Vec<PromptCacheConversationInvocationPreviewResponse>> {
    let mut grouped = HashMap::new();
    for row in rows {
        grouped
            .entry(row.prompt_cache_key.clone())
            .or_insert_with(Vec::new)
            .push(prompt_cache_invocation_preview_from_row(row));
    }
    overlay_runtime_prompt_cache_invocation_previews(
        &mut grouped,
        runtime_overlay_records,
        selected_keys,
        recent_invocation_limit,
    );
    grouped
}

fn log_hydration_duration(
    source_scope: InvocationSourceScope,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
    selected_key_count: usize,
    row_count: usize,
    elapsed_ms: u64,
) {
    let window = if snapshot.is_some() {
        "snapshot"
    } else {
        "live"
    };
    if elapsed_ms >= 250 {
        tracing::warn!(
            endpoint = "/api/prompt-cache/conversations",
            window,
            ?source_scope,
            selected_key_count = selected_key_count as i64,
            row_count = row_count as i64,
            cache_hit_or_miss = "hydrate",
            elapsed_ms,
            "prompt cache conversation hydration exceeded slow-path threshold"
        );
    } else {
        tracing::debug!(
            endpoint = "/api/prompt-cache/conversations",
            window,
            ?source_scope,
            selected_key_count = selected_key_count as i64,
            row_count = row_count as i64,
            cache_hit_or_miss = "hydrate",
            elapsed_ms,
            "prompt cache conversation hydration completed"
        );
    }
}

fn unique_upstream_account_ids_by_name(
    rows: &[PromptCacheConversationUpstreamAccountSummaryRow],
) -> HashMap<String, Option<i64>> {
    let mut unique_ids = HashMap::new();
    for row in rows {
        let Some(name) = normalize_trimmed_optional_string(row.upstream_account_name.clone())
        else {
            continue;
        };
        let Some(account_id) = row.upstream_account_id else {
            continue;
        };
        match unique_ids.entry(name) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some(account_id));
            }
            std::collections::hash_map::Entry::Occupied(mut entry)
                if entry.get().is_some_and(|existing| existing != account_id) =>
            {
                entry.insert(None);
            }
            std::collections::hash_map::Entry::Occupied(_) => {}
        }
    }
    unique_ids
}

fn merge_upstream_accounts_for_key(
    rows: Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
) -> Vec<PromptCacheConversationUpstreamAccountResponse> {
    let unique_ids = unique_upstream_account_ids_by_name(&rows);
    let mut accounts = HashMap::new();
    for row in rows {
        let normalized_name = normalize_trimmed_optional_string(row.upstream_account_name.clone());
        let account_id = row.upstream_account_id.or_else(|| {
            normalized_name
                .as_ref()
                .and_then(|name| unique_ids.get(name).copied().flatten())
        });
        let key =
            resolve_prompt_cache_upstream_account_group_key(account_id, normalized_name.as_deref());
        let entry =
            accounts
                .entry(key)
                .or_insert_with(|| PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: account_id,
                    upstream_account_name: normalized_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: row.last_activity_at.clone(),
                });
        entry.upstream_account_id = entry.upstream_account_id.or(account_id);
        entry.upstream_account_name = entry.upstream_account_name.clone().or(normalized_name);
        entry.request_count += row.request_count;
        entry.total_tokens += row.total_tokens.max(0);
        entry.total_cost += row.total_cost;
        if row.last_activity_at > entry.last_activity_at {
            entry.last_activity_at = row.last_activity_at;
        }
    }
    let mut accounts = accounts.into_values().collect::<Vec<_>>();
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
    accounts
}

fn group_upstream_accounts(
    rows: Vec<PromptCacheConversationUpstreamAccountSummaryRow>,
) -> HashMap<String, Vec<PromptCacheConversationUpstreamAccountResponse>> {
    let mut rows_by_key = HashMap::<String, Vec<_>>::new();
    for row in rows {
        rows_by_key
            .entry(row.prompt_cache_key.clone())
            .or_default()
            .push(row);
    }
    rows_by_key
        .into_iter()
        .map(|(key, rows)| (key, merge_upstream_accounts_for_key(rows)))
        .collect()
}

fn build_hydrated_conversations(
    aggregates: Vec<PromptCacheConversationAggregateRow>,
    mut in_flight_phase_counts_by_key: HashMap<String, InvocationPhaseCountsResponse>,
    mut grouped_events: HashMap<String, Vec<PromptCacheConversationRequestPointResponse>>,
    mut grouped_upstream_accounts: HashMap<
        String,
        Vec<PromptCacheConversationUpstreamAccountResponse>,
    >,
    mut encrypted_owner_rows_by_key: HashMap<
        String,
        PromptCacheConversationEncryptedOwnerSummaryRow,
    >,
    mut manual_binding_by_key: HashMap<String, PromptCacheConversationManualBindingResponse>,
    grouped_recent_invocations: &mut HashMap<
        String,
        Vec<PromptCacheConversationInvocationPreviewResponse>,
    >,
) -> Vec<PromptCacheConversationResponse> {
    aggregates
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
                in_flight_phase_counts: in_flight_phase_counts_by_key
                    .remove(&row.prompt_cache_key)
                    .unwrap_or_default(),
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
        .collect()
}

fn split_hydration_request(
    request: PromptCacheConversationHydrationRequest<'_>,
) -> (
    Vec<PromptCacheConversationAggregateRow>,
    PromptCacheConversationHydrationRequest<'_>,
) {
    let PromptCacheConversationHydrationRequest {
        source_scope,
        aggregates,
        range_end,
        detail_level,
        recent_invocation_limit,
        snapshot,
        runtime_overlay_records,
    } = request;
    (
        aggregates,
        PromptCacheConversationHydrationRequest {
            source_scope,
            aggregates: Vec::new(),
            range_end,
            detail_level,
            recent_invocation_limit,
            snapshot,
            runtime_overlay_records,
        },
    )
}

fn index_hydration_metadata(
    encrypted_owner_rows: Vec<PromptCacheConversationEncryptedOwnerSummaryRow>,
    manual_binding_rows: Vec<PromptCacheConversationManualBindingSummaryRow>,
) -> (
    HashMap<String, PromptCacheConversationEncryptedOwnerSummaryRow>,
    HashMap<String, PromptCacheConversationManualBindingResponse>,
) {
    let encrypted_owner_rows_by_key = encrypted_owner_rows
        .into_iter()
        .map(|row| (row.prompt_cache_key.clone(), row))
        .collect();
    let manual_binding_by_key = manual_binding_rows
        .into_iter()
        .filter_map(prompt_cache_manual_binding_response_from_row)
        .map(|binding| {
            let prompt_cache_key = binding.prompt_cache_key.clone();
            (prompt_cache_key, binding.response)
        })
        .collect();
    (encrypted_owner_rows_by_key, manual_binding_by_key)
}

pub(crate) async fn hydrate_prompt_cache_conversations_on_connection(
    state: &AppState,
    connection: &mut SqliteConnection,
    request: PromptCacheConversationHydrationRequest<'_>,
) -> Result<Vec<PromptCacheConversationResponse>> {
    let (aggregates, request_view) = split_hydration_request(request);
    if aggregates.is_empty() {
        return Ok(Vec::new());
    }

    let started_at = Instant::now();
    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let in_flight_phase_counts_by_key =
        load_in_flight_phase_counts(&mut *connection, &request_view, &selected_keys).await?;
    let recent_invocation_limit = match request_view.detail_level {
        PromptCacheConversationDetailLevel::Full => request_view
            .recent_invocation_limit
            .unwrap_or(PROMPT_CACHE_CONVERSATION_INVOCATION_PREVIEW_LIMIT as i64),
        PromptCacheConversationDetailLevel::Compact => {
            request_view.recent_invocation_limit.unwrap_or(2)
        }
    };

    let chart_range_start_bound =
        (request_view.detail_level == PromptCacheConversationDetailLevel::Full).then(|| {
            resolve_prompt_cache_conversation_chart_range_start(
                request_view.range_end,
                aggregates.iter().map(|row| row.created_at.as_str()).min(),
            )
        });
    let hydration_rows = load_hydration_rows(
        state,
        &mut *connection,
        &request_view,
        &selected_keys,
        recent_invocation_limit,
        chart_range_start_bound.as_deref(),
    )
    .await?;
    let mut events = hydration_rows.events;
    let mut upstream_account_rows = hydration_rows.upstream_account_rows;
    let recent_invocation_rows = hydration_rows.recent_invocation_rows;
    let encrypted_owner_rows = hydration_rows.encrypted_owner_rows;
    let manual_binding_rows = hydration_rows.manual_binding_rows;

    // Runtime records use id 0 until persistence assigns their SQLite identity. The aggregate
    // overlay already selects them, so detail hydration must contribute the same transient rows
    // to charts and account summaries instead of publishing totals without their wire details.
    append_transient_runtime_hydration_rows(
        &request_view,
        &selected_keys,
        chart_range_start_bound.as_deref(),
        &mut events,
        &mut upstream_account_rows,
    );

    let grouped_events = group_hydration_events(events);

    let grouped_upstream_accounts = group_upstream_accounts(upstream_account_rows);
    let mut grouped_recent_invocations = group_recent_invocations(
        recent_invocation_rows,
        request_view.runtime_overlay_records,
        &selected_keys,
        recent_invocation_limit,
    );

    let (encrypted_owner_rows_by_key, manual_binding_by_key) =
        index_hydration_metadata(encrypted_owner_rows, manual_binding_rows);

    let conversations = build_hydrated_conversations(
        aggregates,
        in_flight_phase_counts_by_key,
        grouped_events,
        grouped_upstream_accounts,
        encrypted_owner_rows_by_key,
        manual_binding_by_key,
        &mut grouped_recent_invocations,
    );

    log_hydration_duration(
        request_view.source_scope,
        request_view.snapshot,
        selected_keys.len(),
        conversations.len(),
        started_at.elapsed().as_millis() as u64,
    );

    Ok(conversations)
}
