use super::*;

pub(crate) fn apply_topic_live_overlay_to_payload(
    state: &AppState,
    topic: &SubscriptionTopic,
    payload: &mut Value,
    live: &DashboardActivityLiveSnapshot,
) -> Result<bool, ApiError> {
    if topic.uses_summary_live_overlay() {
        let SubscriptionTopic::SummaryCurrent {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(false);
        };
        let account = upstream_account_id.and_then(|account_id| {
            live.accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(account_id))
        });
        let (count, retry_count, phase_counts, wait_ms) = match account {
            Some(account) => (
                account.in_progress_invocation_count,
                account.retry_invocation_count,
                account.in_progress_phase_counts,
                (account.in_progress_wait_sample_count > 0).then_some(
                    account.in_progress_wait_sum_ms / account.in_progress_wait_sample_count as f64,
                ),
            ),
            None if upstream_account_id.is_some() => {
                (0, 0, InvocationPhaseCountsResponse::default(), None)
            }
            None => (
                live.in_progress_invocation_count,
                live.retry_invocation_count,
                live.in_progress_phase_counts,
                (live.in_progress_wait_sample_count > 0).then_some(
                    live.in_progress_wait_sum_ms / live.in_progress_wait_sample_count as f64,
                ),
            ),
        };
        let Some(object) = payload.as_object_mut() else {
            return Ok(false);
        };
        let next_values = [
            ("inProgressConversationCount", Value::from(count)),
            ("inProgressRetryConversationCount", Value::from(retry_count)),
            (
                "inProgressAvgWaitMs",
                wait_ms.map(Value::from).unwrap_or(Value::Null),
            ),
            ("inProgressPhaseCounts", serde_json::to_value(phase_counts)?),
        ];
        let changed = next_values
            .iter()
            .any(|(key, value)| object.get(*key) != Some(value));
        for (key, value) in next_values {
            set_json_field(object, key, value);
        }
        return Ok(changed);
    }

    if topic.uses_dashboard_activity_live_overlay() {
        return apply_dashboard_activity_live_overlay_to_payload(state, payload, live);
    }

    Ok(false)
}

pub(crate) fn set_json_field(object: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    object.insert(key.to_string(), value);
}

pub(crate) async fn load_persisted_invocation_identities(
    conn: &mut sqlx::SqliteConnection,
    identities: &HashSet<String>,
    baseline_row_id: i64,
) -> Result<HashSet<String>, ApiError> {
    let selectors = identities
        .iter()
        .filter_map(|identity| identity.split_once('\0'))
        .collect::<Vec<_>>();
    let mut persisted = HashSet::new();
    for chunk in selectors.chunks(300) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT invoke_id, occurred_at FROM codex_invocations WHERE id <= ",
        );
        query.push_bind(baseline_row_id);
        query.push(" AND (");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(*invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(*occurred_at)
                .push(")");
        }
        query.push(")");
        for (invoke_id, occurred_at) in query
            .build_query_as::<(String, String)>()
            .fetch_all(&mut *conn)
            .await?
        {
            persisted.insert(format!("{invoke_id}\0{occurred_at}"));
        }
    }
    Ok(persisted)
}

pub(crate) fn patch_prompt_cache_binding_payload(
    payload: &mut Value,
    prompt_cache_key: &str,
    binding: &Value,
) -> Option<bool> {
    let conversation = payload
        .get_mut("conversations")?
        .as_array_mut()?
        .iter_mut()
        .find(|conversation| {
            conversation.get("promptCacheKey").and_then(Value::as_str) == Some(prompt_cache_key)
        })?
        .as_object_mut()?;
    let binding_kind = binding.get("bindingKind").and_then(Value::as_str);
    let manual_binding = binding_kind
        .filter(|kind| *kind != "none")
        .map(|kind| {
            serde_json::json!({
                "bindingKind": kind,
                "groupName": binding.get("groupName").cloned().unwrap_or(Value::Null),
                "upstreamAccountId": binding.get("upstreamAccountId").cloned().unwrap_or(Value::Null),
                "upstreamAccountName": binding.get("upstreamAccountName").cloned().unwrap_or(Value::Null),
            })
        })
        .unwrap_or(Value::Null);
    let replacements = [
        (
            "hasEncryptedSessionOwner",
            binding
                .get("hasEncryptedSessionOwner")
                .cloned()
                .unwrap_or(Value::Bool(false)),
        ),
        (
            "encryptedOwnerAccountId",
            binding
                .get("encryptedOwnerAccountId")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "encryptedOwnerAccountName",
            binding
                .get("encryptedOwnerAccountName")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "encryptedOwnerGroupName",
            binding
                .get("encryptedOwnerGroupName")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("manualBinding", manual_binding),
    ];
    let changed = replacements
        .iter()
        .any(|(field, value)| conversation.get(*field) != Some(value));
    for (field, value) in replacements {
        if value.is_null() {
            conversation.remove(field);
        } else {
            conversation.insert(field.to_string(), value);
        }
    }
    Some(changed)
}

pub(crate) fn apply_prompt_cache_records_to_payload(
    topic: &SubscriptionTopic,
    payload: &mut Value,
    records: &[PromptCacheTopicDelta],
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) -> Result<bool, ApiError> {
    let Some(conversations) = payload
        .as_object_mut()
        .and_then(|object| object.get_mut("conversations"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(false);
    };
    let mut changed = false;
    for record in records {
        let key = match topic {
            SubscriptionTopic::PromptCacheWindow { .. } => record.prompt_cache_key.as_deref(),
            SubscriptionTopic::PromptCacheStickyWindow { .. } => record.sticky_key.as_deref(),
            _ => None,
        };
        let Some(key) = key else {
            continue;
        };
        if let SubscriptionTopic::PromptCacheStickyWindow { account_id, .. } = topic
            && record.upstream_account_id != Some(*account_id)
        {
            continue;
        }
        let key_field = match topic {
            SubscriptionTopic::PromptCacheWindow { .. } => "promptCacheKey",
            SubscriptionTopic::PromptCacheStickyWindow { .. } => "stickyKey",
            _ => return Ok(false),
        };
        let occurred_at = Value::String(record.occurred_at.clone());
        let status = record.status.as_str();
        let is_terminal = record.is_terminal;
        let is_success = record.is_success;
        let request_tokens = record.request_tokens;
        let conversation_index = conversations.iter().position(|conversation| {
            conversation.get(key_field).and_then(Value::as_str) == Some(key)
        });
        if record.is_runtime_removed {
            let Some(index) = conversation_index else {
                continue;
            };
            let Some(conversation) = conversations[index].as_object_mut() else {
                continue;
            };
            let Some(recent) = conversation
                .get_mut("recentInvocations")
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            let count_before = recent.len();
            recent.retain(|item| {
                item.get("invokeId").and_then(Value::as_str) != Some(record.invoke_id.as_str())
                    || item.get("occurredAt") != Some(&occurred_at)
            });
            changed |= recent.len() != count_before;
            continue;
        }
        let Some(preview) = record.preview.as_ref() else {
            continue;
        };
        let preview = serde_json::to_value(preview)?;
        let index = match conversation_index {
            Some(index) => index,
            None => {
                let mut conversation = serde_json::Map::new();
                conversation.insert(key_field.to_string(), Value::String(key.to_string()));
                conversation.insert("requestCount".to_string(), Value::from(0));
                conversation.insert("totalTokens".to_string(), Value::from(0));
                conversation.insert("totalCost".to_string(), Value::from(0.0));
                conversation.insert("createdAt".to_string(), occurred_at.clone());
                conversation.insert("lastActivityAt".to_string(), occurred_at.clone());
                conversation.insert("recentInvocations".to_string(), Value::Array(Vec::new()));
                conversation.insert("last24hRequests".to_string(), Value::Array(Vec::new()));
                if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. }) {
                    conversation.insert("hasEncryptedSessionOwner".to_string(), Value::Bool(false));
                    conversation.insert("upstreamAccounts".to_string(), Value::Array(Vec::new()));
                }
                conversations.push(Value::Object(conversation));
                conversations.len() - 1
            }
        };
        let Some(conversation) = conversations[index].as_object_mut() else {
            continue;
        };
        let recent = conversation
            .entry("recentInvocations")
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("prompt cache recentInvocations is an array");
        let existing_index = recent.iter().position(|item| {
            item.get("invokeId").and_then(Value::as_str)
                == preview.get("invokeId").and_then(Value::as_str)
                && item.get("occurredAt") == Some(&occurred_at)
        });
        let recent_terminal_identity = recent.iter().any(|item| {
            item.get("invokeId").and_then(Value::as_str) == Some(record.invoke_id.as_str())
                && item.get("occurredAt") == Some(&occurred_at)
                && prompt_invocation_status_counts_toward_terminal_totals(
                    item.get("status").and_then(Value::as_str),
                )
        });
        let already_terminal = applied_terminal_ids.contains(&record.identity)
            || (record.row_id > 0 && record.row_id <= baseline_row_id)
            || recent_terminal_identity;
        if is_terminal {
            applied_terminal_ids.insert(record.identity.clone());
        }
        if let Some(index) = existing_index {
            recent[index] = preview.clone();
        } else {
            recent.push(preview.clone());
        }
        recent.sort_by(|left, right| {
            right
                .get("occurredAt")
                .and_then(Value::as_str)
                .cmp(&left.get("occurredAt").and_then(Value::as_str))
        });
        let recent_limit = match topic {
            SubscriptionTopic::PromptCacheWindow {
                recent_invocation_limit,
                ..
            } => recent_invocation_limit.unwrap_or(16).max(0) as usize,
            SubscriptionTopic::PromptCacheStickyWindow { .. } => 5,
            _ => 0,
        };
        recent.truncate(recent_limit);

        let statistics_baseline_is_known_empty =
            conversation.get("requestCount").and_then(Value::as_i64) == Some(0)
                && conversation
                    .get("lastTerminalAt")
                    .and_then(Value::as_str)
                    .is_none();
        if conversation
            .get("lastActivityAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
        {
            conversation.insert("lastActivityAt".to_string(), occurred_at.clone());
        }
        if conversation
            .get("createdAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next < current))
        {
            conversation.insert("createdAt".to_string(), occurred_at.clone());
        }
        if is_terminal && !already_terminal {
            applied_terminal_ids.insert(record.identity.clone());
            increment_json_i64(conversation, "requestCount", 1);
            increment_json_i64(conversation, "totalTokens", request_tokens);
            increment_json_f64(conversation, "totalCost", record.cost);
            if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. }) {
                if conversation
                    .get("lastTerminalAt")
                    .and_then(Value::as_str)
                    .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
                {
                    conversation.insert("lastTerminalAt".to_string(), occurred_at.clone());
                }
                apply_prompt_cache_statistics_delta(
                    conversation,
                    record,
                    &occurred_at,
                    statistics_baseline_is_known_empty,
                );
                apply_prompt_cache_account_delta(conversation, record, &occurred_at);
            }
            let points = conversation
                .entry("last24hRequests")
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("prompt cache last24hRequests is an array");
            if !points.iter().any(|point| {
                point.get("occurredAt") == Some(&occurred_at)
                    && point.get("requestTokens").and_then(Value::as_i64) == Some(request_tokens)
            }) {
                let mut point = serde_json::json!({
                    "occurredAt": occurred_at,
                    "status": status,
                    "isSuccess": is_success,
                    "requestTokens": request_tokens,
                    "cumulativeTokens": 0,
                });
                if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. })
                    && let Some(point) = point.as_object_mut()
                {
                    point.insert(
                        "outcome".to_string(),
                        Value::String(if is_success { "success" } else { "failure" }.to_string()),
                    );
                }
                points.push(point);
                points.sort_by(|left, right| {
                    left.get("occurredAt")
                        .and_then(Value::as_str)
                        .cmp(&right.get("occurredAt").and_then(Value::as_str))
                });
                let mut cumulative = 0_i64;
                for point in points {
                    cumulative = cumulative.saturating_add(
                        point
                            .get("requestTokens")
                            .and_then(Value::as_i64)
                            .unwrap_or_default(),
                    );
                    if let Some(point) = point.as_object_mut() {
                        point.insert("cumulativeTokens".to_string(), Value::from(cumulative));
                    }
                }
            }
        }
        changed = true;
    }

    conversations.sort_by(|left, right| {
        right
            .get("lastActivityAt")
            .and_then(Value::as_str)
            .cmp(&left.get("lastActivityAt").and_then(Value::as_str))
    });
    let display_limit = match topic {
        SubscriptionTopic::PromptCacheWindow { selection, .. } => selection.display_limit(),
        SubscriptionTopic::PromptCacheStickyWindow { selection, .. } => selection.display_limit(),
        _ => 0,
    };
    conversations.truncate(display_limit.max(0) as usize);
    Ok(changed)
}

pub(crate) fn increment_json_i64(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    delta: i64,
) {
    let current = object
        .get(field)
        .and_then(Value::as_i64)
        .unwrap_or_default();
    object.insert(
        field.to_string(),
        Value::from(current.saturating_add(delta)),
    );
}

pub(crate) fn increment_json_optional_i64(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    delta: Option<i64>,
    allow_initialize: bool,
) {
    let Some(delta) = delta else {
        return;
    };
    if allow_initialize || object.get(field).and_then(Value::as_i64).is_some() {
        increment_json_i64(object, field, delta);
    }
}

pub(crate) fn prompt_cache_delta_needs_replay(
    delta: &PromptCacheTopicDelta,
    persisted_identities: &HashSet<String>,
) -> bool {
    !persisted_identities.contains(&delta.identity)
}

pub(crate) fn increment_json_f64(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    delta: f64,
) {
    let current = object
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_default();
    object.insert(field.to_string(), Value::from(current + delta));
}

pub(crate) fn increment_json_optional_f64(
    object: &mut serde_json::Map<String, Value>,
    field: &str,
    delta: Option<f64>,
    allow_initialize: bool,
) {
    let Some(delta) = delta else {
        return;
    };
    if allow_initialize || object.get(field).and_then(Value::as_f64).is_some() {
        increment_json_f64(object, field, delta);
    }
}

pub(crate) fn apply_prompt_cache_account_delta(
    conversation: &mut serde_json::Map<String, Value>,
    record: &PromptCacheTopicDelta,
    occurred_at: &Value,
) {
    let accounts = conversation
        .entry("upstreamAccounts")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("prompt cache upstreamAccounts is an array");
    let index = accounts.iter().position(|account| {
        account.get("upstreamAccountId").and_then(Value::as_i64) == record.upstream_account_id
    });
    let index = index.unwrap_or_else(|| {
        accounts.push(serde_json::json!({
            "upstreamAccountId": record.upstream_account_id,
            "upstreamAccountName": record.upstream_account_name,
            "requestCount": 0,
            "totalTokens": 0,
            "totalCost": 0.0,
            "lastActivityAt": occurred_at,
        }));
        accounts.len() - 1
    });
    if let Some(account) = accounts[index].as_object_mut() {
        increment_json_i64(account, "requestCount", 1);
        increment_json_i64(account, "totalTokens", record.request_tokens);
        increment_json_f64(account, "totalCost", record.cost);
        if account
            .get("lastActivityAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
        {
            account.insert("lastActivityAt".to_string(), occurred_at.clone());
        }
    }
}

pub(crate) fn apply_prompt_cache_statistics_delta(
    conversation: &mut serde_json::Map<String, Value>,
    record: &PromptCacheTopicDelta,
    occurred_at: &Value,
    statistics_baseline_is_known_empty: bool,
) {
    increment_json_optional_i64(
        conversation,
        "successCount",
        Some(i64::from(record.is_success)),
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_i64(
        conversation,
        "failureCount",
        Some(i64::from(!record.is_success)),
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_i64(
        conversation,
        "inputTokens",
        record.input_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_i64(
        conversation,
        "outputTokens",
        record.output_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_i64(
        conversation,
        "cacheInputTokens",
        record.cache_input_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_i64(
        conversation,
        "reportedCacheWriteTokens",
        record.reported_cache_write_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_i64(
        conversation,
        "reasoningTokens",
        record.reasoning_tokens,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_f64(
        conversation,
        "costInput",
        record.cost_input,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_f64(
        conversation,
        "costCacheWrite",
        record.cost_cache_write,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_f64(
        conversation,
        "costCacheRead",
        record.cost_cache_read,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_f64(
        conversation,
        "costOutput",
        record.cost_output,
        statistics_baseline_is_known_empty,
    );
    increment_json_optional_f64(
        conversation,
        "costReasoning",
        record.cost_reasoning,
        statistics_baseline_is_known_empty,
    );

    let occurred_at = occurred_at.as_str().unwrap_or_default();
    let first_invocation_at = conversation
        .get("firstInvocationAt")
        .and_then(Value::as_str);
    if (statistics_baseline_is_known_empty || first_invocation_at.is_some())
        && first_invocation_at.is_none_or(|current| occurred_at < current)
    {
        conversation.insert(
            "firstInvocationAt".to_string(),
            Value::String(occurred_at.to_string()),
        );
    }
    let last_invocation_at = conversation.get("lastInvocationAt").and_then(Value::as_str);
    if (statistics_baseline_is_known_empty || last_invocation_at.is_some())
        && last_invocation_at.is_none_or(|current| occurred_at > current)
    {
        conversation.insert(
            "lastInvocationAt".to_string(),
            Value::String(occurred_at.to_string()),
        );
    }
}

pub(crate) fn set_json_optional_field(
    object: &mut serde_json::Map<String, Value>,
    key: &str,
    value: Option<Value>,
) {
    match value {
        Some(value) => {
            object.insert(key.to_string(), value);
        }
        None => {
            object.remove(key);
        }
    }
}

pub(crate) fn dashboard_activity_payload_exact_range(payload: &Value) -> Option<ExactUtcRange> {
    let object = payload.as_object()?;
    let range_start = object.get("rangeStart")?.as_str()?;
    let range_end = object.get("rangeEnd")?.as_str()?;
    Some(ExactUtcRange {
        start: parse_to_utc_datetime(range_start)?,
        end: parse_to_utc_datetime(range_end)?,
    })
}

pub(crate) async fn dashboard_activity_snapshot_selection_for_topic(
    state: &AppState,
    topic: &SubscriptionTopic,
) -> Result<Option<DashboardActivitySnapshotSelection>, ApiError> {
    let SubscriptionTopic::DashboardActivityCurrent {
        range,
        time_zone,
        recent_limit,
        include_accounts,
        include_recent,
    } = topic
    else {
        return Ok(None);
    };

    if range == "yesterday" {
        return Ok(None);
    }

    let recent_limit = validate_dashboard_activity_params(
        "dashboard.activity.current",
        range,
        Some(*recent_limit),
    )?;
    let reporting_tz = parse_reporting_tz(Some(time_zone))?;
    let exact_range = resolve_dashboard_activity_cached_range(range, reporting_tz)?;
    let source_scope = resolve_default_source_scope(&state.pool).await?;

    Ok(Some(build_dashboard_activity_snapshot_selection(
        range,
        exact_range,
        reporting_tz,
        source_scope,
        recent_limit,
        *include_accounts,
        *include_recent,
    )))
}

pub(crate) fn dashboard_activity_account_sort_tuple(value: &Value) -> (i64, Option<&str>, i64) {
    let total_tokens = value
        .get("totalTokens")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let first_recent_occurred_at = value
        .get("recentInvocations")
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("occurredAt"))
        .and_then(Value::as_str);
    let upstream_account_id = value
        .get("upstreamAccountId")
        .and_then(Value::as_i64)
        .unwrap_or(i64::MIN);
    (total_tokens, first_recent_occurred_at, upstream_account_id)
}

pub(crate) fn apply_dashboard_activity_live_overlay_to_payload(
    state: &AppState,
    payload: &mut Value,
    live: &DashboardActivityLiveSnapshot,
) -> Result<bool, ApiError> {
    let request_range = dashboard_activity_payload_exact_range(payload);
    let Some(root) = payload.as_object_mut() else {
        return Ok(false);
    };
    let model_performance_available = root
        .get("summary")
        .and_then(Value::as_object)
        .and_then(|summary| summary.get("modelPerformance"))
        .and_then(Value::as_object)
        .and_then(|model| model.get("available"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    root.insert("liveRevision".to_string(), json!(live.revision));
    set_json_optional_field(
        root,
        "networkLiveBucket",
        live.network_live_bucket
            .clone()
            .map(serde_json::to_value)
            .transpose()?,
    );
    set_json_optional_field(
        root,
        "networkRealtimeRate",
        live.network_realtime_rate
            .clone()
            .map(serde_json::to_value)
            .transpose()?,
    );
    let current_snapshot_by_account = state
        .dashboard_network_speed_cache
        .snapshot_dashboard_activity_accounts(Utc::now());
    let current_snapshot_summary =
        sum_dashboard_activity_current_snapshots(current_snapshot_by_account.values().copied());
    {
        let Some(summary) = root.get_mut("summary").and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        let Some(stats) = summary.get_mut("stats").and_then(Value::as_object_mut) else {
            return Ok(false);
        };
        stats.insert(
            "inProgressConversationCount".to_string(),
            json!(live.in_progress_invocation_count),
        );
        stats.insert(
            "inProgressRetryConversationCount".to_string(),
            json!(live.retry_invocation_count),
        );
        stats.insert(
            "inProgressPhaseCounts".to_string(),
            serde_json::to_value(live.in_progress_phase_counts)?,
        );
        summary.insert(
            "tokensPerMinute".to_string(),
            json!(current_snapshot_summary.qualified_tokens.max(0) as f64),
        );
        summary.insert(
            "spendRate".to_string(),
            json!(current_snapshot_summary.total_cost.max(0.0)),
        );
        set_json_optional_field(
            summary,
            "currentFirstResponseByteTotalAvgMs",
            current_snapshot_summary
                .first_response_byte_total_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            summary,
            "currentFirstTokenAvgMs",
            current_snapshot_summary
                .first_token_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            summary,
            "currentAvgTotalMs",
            current_snapshot_summary
                .avg_total_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            summary,
            "currentAvgResponseMs",
            current_snapshot_summary
                .avg_response_duration_ms()
                .map(|value| json!(value)),
        );
    }

    let Some(accounts_value) = root.get_mut("accounts") else {
        return Ok(true);
    };
    let Some(accounts) = accounts_value.as_array_mut() else {
        return Ok(true);
    };

    let live_accounts = live
        .accounts
        .iter()
        .map(|account| (account.account_key.as_str(), account))
        .collect::<HashMap<_, _>>();
    let mut existing_account_keys = HashSet::new();

    for account in accounts.iter_mut() {
        let Some(account_object) = account.as_object_mut() else {
            continue;
        };
        let Some(account_key) = account_object.get("accountKey").and_then(Value::as_str) else {
            continue;
        };
        existing_account_keys.insert(account_key.to_string());
        let live_account = live_accounts.get(account_key).copied();
        let upstream_account_id = account_object
            .get("upstreamAccountId")
            .and_then(Value::as_i64);
        let current_snapshot = current_snapshot_by_account
            .get(&upstream_account_id)
            .copied()
            .unwrap_or_default();

        set_json_field(
            account_object,
            "inProgressInvocationCount",
            json!(live_account.map_or(0, |account| account.in_progress_invocation_count)),
        );
        set_json_field(
            account_object,
            "inProgressPhaseCounts",
            serde_json::to_value(
                live_account
                    .map(|account| account.in_progress_phase_counts)
                    .unwrap_or_default(),
            )?,
        );
        set_json_field(
            account_object,
            "retryInvocationCount",
            json!(live_account.map_or(0, |account| account.retry_invocation_count)),
        );
        set_json_field(
            account_object,
            "uploadBytesPerSecond",
            json!(live_account.map_or(0.0, |account| account.upload_bytes_per_second)),
        );
        set_json_field(
            account_object,
            "downloadBytesPerSecond",
            json!(live_account.map_or(0.0, |account| account.download_bytes_per_second)),
        );
        set_json_field(
            account_object,
            "tokensPerMinute",
            json!(current_snapshot.qualified_tokens.max(0) as f64),
        );
        set_json_field(
            account_object,
            "spendRate",
            json!(current_snapshot.total_cost.max(0.0)),
        );
        set_json_optional_field(
            account_object,
            "currentFirstResponseByteTotalAvgMs",
            current_snapshot
                .first_response_byte_total_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            account_object,
            "currentFirstTokenAvgMs",
            current_snapshot
                .first_token_avg_ms()
                .map(|value| json!(value)),
        );
        set_json_optional_field(
            account_object,
            "currentAvgTotalMs",
            current_snapshot.avg_total_ms().map(|value| json!(value)),
        );
        set_json_optional_field(
            account_object,
            "currentAvgResponseMs",
            current_snapshot
                .avg_response_duration_ms()
                .map(|value| json!(value)),
        );
        if let Some(live_account) = live_account {
            let current_request_count = account_object
                .get("requestCount")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            set_json_field(
                account_object,
                "requestCount",
                json!(current_request_count.max(live_account.in_progress_invocation_count.max(0))),
            );
        }
    }

    if let Some(request_range) = request_range {
        for live_account in &live.accounts {
            if existing_account_keys.contains(&live_account.account_key) {
                continue;
            }
            let current_snapshot = current_snapshot_by_account
                .get(&live_account.upstream_account_id)
                .copied()
                .unwrap_or_default();
            let placeholder = dashboard_activity_account_from_live(
                live_account,
                None,
                request_range,
                current_snapshot,
                model_performance_available,
                None,
                Vec::new(),
            );
            accounts.push(serde_json::to_value(placeholder)?);
        }
        accounts.sort_by(|left, right| {
            let left_key = dashboard_activity_account_sort_tuple(left);
            let right_key = dashboard_activity_account_sort_tuple(right);
            right_key
                .0
                .cmp(&left_key.0)
                .then_with(|| right_key.1.cmp(&left_key.1))
                .then_with(|| right_key.2.cmp(&left_key.2))
        });
    }

    Ok(true)
}

pub(crate) fn dashboard_activity_response_exact_range(
    response: &DashboardActivityResponse,
) -> Option<ExactUtcRange> {
    Some(ExactUtcRange {
        start: parse_to_utc_datetime(&response.range_start)?,
        end: parse_to_utc_datetime(&response.range_end)?,
    })
}

pub(crate) fn apply_dashboard_activity_slices(
    response: &mut DashboardActivityResponse,
    current: Option<&DashboardCurrentProjectionSlice>,
    network: Option<&DashboardNetworkProjectionSlice>,
) {
    if let Some(current) = current {
        response.live_revision = current.revision;
        response.summary.stats.in_progress_conversation_count =
            Some(current.in_progress_invocation_count);
        response.summary.stats.in_progress_retry_conversation_count =
            Some(current.retry_invocation_count);
        response.summary.stats.in_progress_avg_wait_ms = (current.in_progress_wait_sample_count
            > 0)
        .then_some(current.in_progress_wait_sum_ms / current.in_progress_wait_sample_count as f64);
        response.summary.stats.in_progress_phase_counts = Some(current.in_progress_phase_counts);

        let exact_range = dashboard_activity_response_exact_range(response);
        let model_performance_available = response.summary.model_performance.available;
        if let Some(accounts) = response.accounts.as_mut() {
            let current_by_key = current
                .accounts
                .iter()
                .map(|account| (account.account_key.as_str(), account))
                .collect::<HashMap<_, _>>();
            let existing_account_keys = accounts
                .iter()
                .map(|account| account.account_key.clone())
                .collect::<HashSet<_>>();

            for account in accounts.iter_mut() {
                apply_dashboard_current_slice_to_activity_account(
                    account,
                    current_by_key.get(account.account_key.as_str()).copied(),
                );
            }

            if let Some(exact_range) = exact_range {
                for account in &current.accounts {
                    if existing_account_keys.contains(&account.account_key) {
                        continue;
                    }
                    let live_account = DashboardActivityLiveAccount {
                        account_key: account.account_key.clone(),
                        upstream_account_id: account.upstream_account_id,
                        upstream_account_name: account.upstream_account_name.clone(),
                        in_progress_invocation_count: account.in_progress_invocation_count,
                        in_progress_phase_counts: account.in_progress_phase_counts,
                        retry_invocation_count: account.retry_invocation_count,
                        in_progress_wait_sum_ms: account.in_progress_wait_sum_ms,
                        in_progress_wait_sample_count: account.in_progress_wait_sample_count,
                        upload_bytes_per_second: 0.0,
                        download_bytes_per_second: 0.0,
                        network_live_bucket: None,
                    };
                    accounts.push(dashboard_activity_account_from_live(
                        &live_account,
                        None,
                        exact_range,
                        network
                            .and_then(|slice| {
                                slice
                                    .current_snapshot_by_account
                                    .get(&account.upstream_account_id)
                                    .copied()
                            })
                            .unwrap_or_default(),
                        model_performance_available,
                        None,
                        Vec::new(),
                    ));
                }
                sort_dashboard_activity_accounts(accounts);
            }
        }
    }

    if let Some(network) = network {
        response.network_live_bucket = network.network_live_bucket.clone();
        response.network_realtime_rate = network.network_realtime_rate.clone();
        response.summary.tokens_per_minute =
            Some(network.current_snapshot.qualified_tokens.max(0) as f64);
        response.summary.spend_rate = Some(network.current_snapshot.total_cost.max(0.0));
        response.summary.current_first_response_byte_total_avg_ms =
            network.current_snapshot.first_response_byte_total_avg_ms();
        response.summary.current_first_token_avg_ms = network.current_snapshot.first_token_avg_ms();
        response.summary.current_avg_total_ms = network.current_snapshot.avg_total_ms();
        response.summary.current_avg_response_ms =
            network.current_snapshot.avg_response_duration_ms();
        if let Some(accounts) = response.accounts.as_mut() {
            let network_by_account = network
                .accounts
                .iter()
                .map(|account| (account.upstream_account_id, account))
                .collect::<HashMap<_, _>>();
            for account in accounts {
                let network_account = network_by_account
                    .get(&account.upstream_account_id)
                    .copied();
                account.upload_bytes_per_second =
                    network_account.map_or(0.0, |value| value.upload_bytes_per_second);
                account.download_bytes_per_second =
                    network_account.map_or(0.0, |value| value.download_bytes_per_second);
                apply_dashboard_current_rate_to_activity_account(
                    account,
                    network
                        .current_snapshot_by_account
                        .get(&account.upstream_account_id)
                        .copied()
                        .unwrap_or_default(),
                );
            }
        }
    }
}

pub(crate) fn apply_dashboard_current_slice_to_activity_account(
    account: &mut DashboardActivityAccountResponse,
    current: Option<&DashboardCurrentProjectionAccountSlice>,
) {
    account.in_progress_invocation_count =
        Some(current.map_or(0, |value| value.in_progress_invocation_count));
    account.in_progress_phase_counts = Some(
        current
            .map(|value| value.in_progress_phase_counts)
            .unwrap_or_default(),
    );
    account.retry_invocation_count = Some(current.map_or(0, |value| value.retry_invocation_count));
    if let Some(current) = current {
        account.request_count = account
            .request_count
            .max(current.in_progress_invocation_count.max(0));
    }
}

pub(crate) fn apply_dashboard_current_rate_to_activity_account(
    account: &mut DashboardActivityAccountResponse,
    current: DashboardActivityCurrentSnapshot,
) {
    account.tokens_per_minute = Some(current.qualified_tokens.max(0) as f64);
    account.spend_rate = Some(current.total_cost.max(0.0));
    account.current_first_response_byte_total_avg_ms = current.first_response_byte_total_avg_ms();
    account.current_first_token_avg_ms = current.first_token_avg_ms();
    account.current_avg_total_ms = current.avg_total_ms();
    account.current_avg_response_ms = current.avg_response_duration_ms();
}

pub(crate) fn apply_dashboard_current_slice_to_summary_response(
    response: &mut StatsResponse,
    upstream_account_id: Option<i64>,
    current: Option<&DashboardCurrentProjectionSlice>,
) {
    let Some(current) = current else {
        return;
    };
    let account = upstream_account_id.and_then(|account_id| {
        current
            .accounts
            .iter()
            .find(|account| account.upstream_account_id == Some(account_id))
    });
    let (count, retry_count, phase_counts, wait_ms) = match account {
        Some(account) => (
            account.in_progress_invocation_count,
            account.retry_invocation_count,
            account.in_progress_phase_counts,
            (account.in_progress_wait_sample_count > 0).then_some(
                account.in_progress_wait_sum_ms / account.in_progress_wait_sample_count as f64,
            ),
        ),
        None if upstream_account_id.is_some() => {
            (0, 0, InvocationPhaseCountsResponse::default(), None)
        }
        None => (
            current.in_progress_invocation_count,
            current.retry_invocation_count,
            current.in_progress_phase_counts,
            (current.in_progress_wait_sample_count > 0).then_some(
                current.in_progress_wait_sum_ms / current.in_progress_wait_sample_count as f64,
            ),
        ),
    };
    response.in_progress_conversation_count = Some(count);
    response.in_progress_retry_conversation_count = Some(retry_count);
    response.in_progress_avg_wait_ms = wait_ms;
    response.in_progress_phase_counts = Some(phase_counts);
}

pub(crate) fn terminal_delta_matches_source_scope(
    delta: &DashboardActivityTerminalDelta,
    source_scope: InvocationSourceScope,
) -> bool {
    source_scope != InvocationSourceScope::ProxyOnly || delta.source == SOURCE_PROXY
}

pub(crate) fn terminal_delta_is_within_range(
    delta: &DashboardActivityTerminalDelta,
    range: ExactUtcRange,
) -> bool {
    parse_to_utc_datetime(&delta.occurred_at)
        .is_some_and(|occurred_at| occurred_at >= range.start && occurred_at < range.end)
}

pub(crate) fn apply_dashboard_terminal_slice_to_summary_response(
    response: &mut StatsResponse,
    terminal_sequence: &mut u64,
    window: &SummaryWindow,
    reporting_tz: Tz,
    source_scope: InvocationSourceScope,
    upstream_account_id: Option<i64>,
    slice: &DashboardTerminalProjectionSlice,
) {
    let range = summary_window_range(window, reporting_tz, Utc::now())
        .ok()
        .flatten()
        .map(|(start, end)| ExactUtcRange { start, end });
    for delta in &slice.deltas {
        if !terminal_delta_matches_source_scope(delta, source_scope)
            // Replayed durable rows deliberately use sequence zero because their original
            // process-local sequence is not trustworthy after restart.  Their full source
            // identity is the dedupe key, so they must still be applied to range responses.
            || (delta.terminal_sequence != 0 && delta.terminal_sequence <= *terminal_sequence)
            || upstream_account_id
                .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
            || range.is_some_and(|range| !terminal_delta_is_within_range(delta, range))
        {
            continue;
        }
        apply_dashboard_activity_terminal_delta_to_stats(response, delta);
        *terminal_sequence = (*terminal_sequence).max(delta.terminal_sequence);
    }
}

pub(crate) fn dashboard_network_timeseries_live_point<'a>(
    base: &'a DashboardNetworkTimeseriesResponse,
    upstream_account_id: Option<i64>,
    network: Option<&'a DashboardNetworkProjectionSlice>,
) -> Option<(usize, &'a DashboardNetworkTimeseriesPointResponse)> {
    let network = network?;
    let bucket = match upstream_account_id {
        None => network.network_live_bucket.as_ref(),
        Some(upstream_account_id) => network
            .accounts
            .iter()
            .find(|account| account.upstream_account_id == Some(upstream_account_id))
            .and_then(|account| account.network_live_bucket.as_ref()),
    };
    let bucket = bucket?;
    let bucket_start = &bucket.bucket_start;
    base.points
        .iter()
        .position(|point| point.bucket_start == *bucket_start)
        .or_else(|| base.points.iter().position(|point| point.is_live_bucket))
        .map(|point_index| (point_index, bucket))
}

pub(crate) async fn wait_for_prompt_cache_reconcile_eligibility(
    gate: &DbPressureGate,
    observed_eligibility: u64,
    reason: DbPressureDenyReason,
) {
    match reason {
        DbPressureDenyReason::PressureCooldown { remaining_ms } => {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(remaining_ms.max(1))) => {}
                _ = gate.wait_for_eligibility_change(observed_eligibility) => {}
            }
        }
        DbPressureDenyReason::BackgroundBusy => {
            gate.wait_for_eligibility_change(observed_eligibility).await;
        }
    }
}

pub(crate) async fn run_server_push_topic_loop(
    hub: Arc<SubscriptionHub>,
    state: Arc<AppState>,
    topic_key: String,
    topic: SubscriptionTopic,
) {
    if matches!(
        topic,
        SubscriptionTopic::PromptCacheWindow { .. }
            | SubscriptionTopic::PromptCacheStickyWindow { .. }
            | SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
    ) {
        let mut interval = tokio::time::interval(PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    hub.clear_server_push_task(&topic_key).await;
                    break;
                }
                _ = interval.tick() => {
                    if hub.stop_server_push_task_if_idle(&topic_key).await {
                        break;
                    }
                    let expiry_result = if matches!(
                        &topic,
                        SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
                    ) {
                        hub.expire_dashboard_working_conversations_projection(state.clone(), &topic_key)
                            .await
                    } else {
                        hub.expire_prompt_cache_topic_window(&topic_key).await
                    };
                    if let Err(err) = expiry_result {
                        warn!(?err, topic = %topic.name(), "failed to expire prompt cache topic window");
                    }
                    if !hub.prompt_cache_reconcile_required(&topic_key).await {
                        continue;
                    }
                    if !hub.begin_prompt_cache_topic_reconcile(&topic).await {
                        continue;
                    }
                    SubscriptionHub::spawn_prompt_cache_topic_reconcile(state.clone(), topic.clone());
                }
            }
        }
        return;
    }
    if !topic.is_closed_summary_topic() {
        let mut interval = tokio::time::interval(DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        interval.tick().await;

        loop {
            tokio::select! {
                _ = state.shutdown.cancelled() => {
                    hub.clear_server_push_task(&topic_key).await;
                    break;
                }
                _ = interval.tick() => {
                    if hub.stop_server_push_task_if_idle(&topic_key).await {
                        break;
                    }
                    if let Err(err) = hub
                        .refresh_topic_if_active(state.clone(), topic.clone(), true)
                        .await
                    {
                        warn!(?err, topic = %topic.name(), "failed to push legacy network recent topic cadence");
                    }
                }
            }
        }
        return;
    }

    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                hub.clear_server_push_task(&topic_key).await;
                break;
            }
            _ = tokio::time::sleep(subscription_calendar_rollover_delay(&topic)) => {
                if hub.stop_server_push_task_if_idle(&topic_key).await {
                    break;
                }
                if let Err(err) = hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await
                {
                    warn!(?err, topic = %topic.name(), "failed to refresh closed summary topic at calendar rollover");
                }
            }
        }
    }
}

pub(crate) fn spawn_subscription_broadcast_listener(state: Arc<AppState>) {
    let hub = state.subscription_hub.clone();
    let shutdown = state.shutdown.clone();
    let mut receiver = state.broadcaster.subscribe();
    hub.mark_internal_broadcast_listener_started();
    let listener_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                item = receiver.recv() => {
                    match item {
                        Ok(payload) => hub.handle_internal_broadcast(listener_state.clone(), payload).await,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            hub.mark_internal_broadcast_gap_and_recover(
                                listener_state.clone(),
                                skipped,
                            )
                            .await;
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        }
    });
    spawn_runtime_mutation_router(state);
}

pub(crate) fn spawn_runtime_mutation_router(state: Arc<AppState>) {
    let hub = state.subscription_hub.clone();
    let bus = hub.runtime_mutation_bus();
    if !bus.claim_router() {
        return;
    }
    let shutdown = state.shutdown.clone();
    let mut receiver = bus.subscribe();
    tokio::spawn(async move {
        let mut last_sequence = 0_u64;
        loop {
            let first = tokio::select! {
                _ = shutdown.cancelled() => return,
                item = receiver.recv() => item,
            };
            let first = match first {
                Ok(first) => first,
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    bus.record_router_lag();
                    bus.record_router_gap();
                    bus.record_cursor_recovery();
                    hub.mark_runtime_mutation_gap_and_recover(
                        state.clone(),
                        skipped,
                        "receiver_lagged",
                    )
                    .await;
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            let mut batch = vec![first];
            let mut lagged = 0_u64;
            while batch.len() < RUNTIME_MUTATION_ROUTER_MAX_BATCH {
                match receiver.try_recv() {
                    Ok(mutation) => batch.push(mutation),
                    Err(broadcast::error::TryRecvError::Empty) => break,
                    Err(broadcast::error::TryRecvError::Lagged(skipped)) => {
                        lagged = lagged.saturating_add(skipped);
                        break;
                    }
                    Err(broadcast::error::TryRecvError::Closed) => break,
                }
            }
            if lagged > 0 {
                bus.record_router_lag();
                bus.record_router_gap();
                bus.record_cursor_recovery();
                hub.mark_runtime_mutation_gap_and_recover(state.clone(), lagged, "receiver_lagged")
                    .await;
                continue;
            }
            if runtime_mutation_batch_has_sequence_gap(&mut last_sequence, &batch) {
                bus.record_router_gap();
                bus.record_cursor_recovery();
                hub.mark_runtime_mutation_gap_and_recover(state.clone(), lagged, "cursor_gap")
                    .await;
                continue;
            }
            hub.handle_runtime_mutation_batch(state.clone(), batch)
                .await;
        }
    });
}

pub(crate) fn schedule_managed_task_topic_refresh_retry(
    hub: Arc<SubscriptionHub>,
    state: Arc<AppState>,
    topic: SubscriptionTopic,
    delay: Duration,
) {
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        if !hub.mark_managed_task_topic_dirty(&topic).await {
            return;
        }
        if let Err(error) = hub
            .refresh_topic_if_active(state.clone(), topic.clone(), true)
            .await
        {
            warn!(
                topic = topic.name(),
                ?error,
                "managed task SSE topic retry failed"
            );
            schedule_managed_task_topic_refresh_retry(
                hub,
                state,
                topic,
                std::cmp::min(delay.saturating_mul(2), Duration::from_secs(30)),
            );
        }
    });
}

pub(crate) fn runtime_mutation_batch_has_sequence_gap(
    last_sequence: &mut u64,
    batch: &[SequencedRuntimeMutation],
) -> bool {
    let mut gap = false;
    for mutation in batch {
        if mutation.sequence != last_sequence.saturating_add(1) {
            gap = true;
        }
        *last_sequence = mutation.sequence;
    }
    gap
}

pub(crate) async fn topic_sse_stream(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SubscriptionStreamQuery>,
) -> Result<Response, ApiError> {
    let descriptors = decode_topics_query(query.topics.as_deref())?;
    let resume = decode_resume_query(query.resume.as_deref(), &descriptors)?;
    let resume_count = resume.len();
    let mut live_receiver = state.subscription_hub.subscribe();
    let selected_topics = descriptors
        .iter()
        .map(SubscriptionTopic::from_descriptor)
        .collect::<Result<Vec<_>, _>>()?;
    let selected_task_topics = selected_topics
        .iter()
        .filter(|topic| {
            matches!(
                topic,
                SubscriptionTopic::ManagedTaskCatalog
                    | SubscriptionTopic::ManagedTaskRuntime
                    | SubscriptionTopic::ManagedTaskTimeline
                    | SubscriptionTopic::ManagedTaskTimelineV2
                    | SubscriptionTopic::ManagedTaskDetail { .. }
                    | SubscriptionTopic::ManagedTaskWorkload { .. }
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let task_change_receiver =
        (!selected_task_topics.is_empty()).then(crate::task_timeline::subscribe_changes);
    let selected_topic_keys = selected_topics
        .iter()
        .map(SubscriptionTopic::cache_key)
        .collect::<Result<HashSet<_>, _>>()?;
    let selected_dashboard_topology_topic_names = selected_topics
        .iter()
        .map(SubscriptionTopic::name)
        .filter(|topic_name| {
            matches!(
                *topic_name,
                "dashboard.activity.current"
                    | "stats.summary.current"
                    | "dashboard.network-timeseries.window"
                    | "dashboard.network-recent.current"
                    | "dashboard.working-conversations.current"
                    | "stats.parallel-work.current"
                    | "stats.timeseries.open-window"
            )
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    #[cfg(test)]
    let dashboard_topology_observer_attempt = (query.reason.as_deref()
        == Some(DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON))
    .then_some(query.attempt)
    .flatten();
    let dashboard_topology_hub = state.subscription_hub.clone();
    let topic_lease = state
        .subscription_hub
        .register_topic_subscribers(&selected_topics)
        .await?;
    let prepared = state
        .subscription_hub
        .prepare_connection_with_capability(
            state.clone(),
            descriptors,
            resume,
            query.delta == Some(1),
        )
        .await?;
    if selected_topics.iter().any(|topic| {
        topic.uses_dashboard_activity_live_overlay()
            || topic.uses_summary_live_overlay()
            || topic.uses_timeseries_live_projection()
            || topic.uses_dashboard_network_live_snapshot()
    }) {
        ensure_dashboard_activity_live_snapshot_producer(state.as_ref());
    }
    tracing::info!(
        attempt = query.attempt,
        reason = query.reason.as_deref().unwrap_or("unknown"),
        topic_count = selected_topic_keys.len(),
        resume_count,
        init_outcomes = ?prepared.outcomes,
        "subscription connection prepared"
    );
    let PreparedSubscriptionConnection {
        initial,
        last_sent_cursors: last_seen_by_topic,
        outcomes: _,
    } = prepared;
    let runtime_projection_mode = state.proxy_runtime_invocations.mode();
    let server_push_topics = selected_topics
        .iter()
        .filter(|topic| topic.uses_server_push_cadence(runtime_projection_mode))
        .cloned()
        .collect::<Vec<_>>();
    let server_push_lease = state
        .subscription_hub
        .register_server_push_topics(state.clone(), server_push_topics)
        .await?;

    let initial_stream = stream::iter(initial.into_iter().flat_map(|prepared| {
        prepared
            .frame
            .event_chunks(prepared.kind)
            .map(Ok::<_, Infallible>)
    }));

    let live_stream = async_stream::stream! {
        let _topic_lease = topic_lease;
        let _server_push_lease = server_push_lease;
        let mut task_changes = task_change_receiver;
        let mut last_seen = last_seen_by_topic;
        let mut keep_alive = tokio::time::interval(Duration::from_secs(15));
        keep_alive.tick().await;
        loop {
            tokio::select! {
                _ = keep_alive.tick() => yield Ok::<_, Infallible>(Bytes::from_static(b":\n\n")),
                changed = next_task_observation_change(&mut task_changes) => match changed {
                    Ok(change) => {
                        for topic in selected_task_topics.iter().filter(|topic| {
                            managed_task_change_matches_topic(&change, topic)
                        }) {
                            if !dashboard_topology_hub
                                .mark_managed_task_topic_dirty(topic)
                                .await
                            {
                                continue;
                            }
                            if let Err(error) = dashboard_topology_hub
                                .refresh_topic_if_active(state.clone(), topic.clone(), true)
                                .await
                            {
                                warn!(topic = topic.name(), ?error, "managed task SSE topic refresh failed; scheduling retry");
                                schedule_managed_task_topic_refresh_retry(
                                    dashboard_topology_hub.clone(),
                                    state.clone(),
                                    topic.clone(),
                                    Duration::from_secs(1),
                                );
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "managed task SSE topic change receiver lagged");
                        for topic in &selected_task_topics {
                            if !dashboard_topology_hub
                                .mark_managed_task_topic_dirty(topic)
                                .await
                            {
                                continue;
                            }
                            if let Err(error) = dashboard_topology_hub
                                .refresh_topic_if_active(state.clone(), topic.clone(), true)
                                .await
                            {
                                warn!(topic = topic.name(), ?error, "managed task SSE topic recovery failed; scheduling retry");
                                schedule_managed_task_topic_refresh_retry(
                                    dashboard_topology_hub.clone(),
                                    state.clone(),
                                    topic.clone(),
                                    Duration::from_secs(1),
                                );
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => task_changes = None,
                },
                received = live_receiver.recv() => match received {
                    Ok(dispatch) => {
                        if !selected_topic_keys.contains(&dispatch.frame.topic_key) {
                            continue;
                        }
                        let previous_cursor = last_seen.get(&dispatch.frame.topic_key).copied().unwrap_or(0);
                        if dispatch.frame.cursor <= previous_cursor {
                            continue;
                        }
                        last_seen.insert(dispatch.frame.topic_key.clone(), dispatch.frame.cursor);
                        let frame = if query.delta == Some(1) {
                            &dispatch.frame
                        } else {
                            &dispatch.full_frame
                        };
                        dashboard_topology_hub
                            .record_dashboard_topology_frame_delivery(frame);
                        #[cfg(test)]
                        if let Some(attempt) = dashboard_topology_observer_attempt {
                            dashboard_topology_hub
                                .record_dashboard_topology_sse_frame_delivery(attempt, frame)
                                .await;
                        }
                        for chunk in frame.event_chunks(TopicFrameKind::Live) {
                            yield Ok::<_, Infallible>(chunk);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        dashboard_topology_hub.record_dashboard_topology_lag(
                            &selected_dashboard_topology_topic_names,
                            skipped,
                        );
                        warn!(skipped, "subscription live fanout lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    };

    let merged = initial_stream.chain(live_stream);
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(merged))
        .map_err(|err| ApiError::from(anyhow!(err)))
}
