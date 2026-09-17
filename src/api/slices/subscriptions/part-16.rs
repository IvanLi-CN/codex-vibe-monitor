fn apply_topic_live_overlay_to_payload(
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

fn set_json_field(object: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    object.insert(key.to_string(), value);
}

async fn load_persisted_invocation_identities(
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

fn patch_prompt_cache_binding_payload(
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

fn apply_prompt_cache_records_to_payload(
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
        changed |= apply_prompt_cache_record_to_conversations(
            topic,
            conversations,
            record,
            applied_terminal_ids,
            baseline_row_id,
        )?;
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

fn apply_prompt_cache_record_to_conversations(
    topic: &SubscriptionTopic,
    conversations: &mut Vec<Value>,
    record: &PromptCacheTopicDelta,
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) -> Result<bool, ApiError> {
    let Some((key_field, key)) = prompt_cache_record_target(topic, record) else {
        return Ok(false);
    };
    let occurred_at = Value::String(record.occurred_at.clone());
    let conversation_index = conversations
        .iter()
        .position(|conversation| conversation.get(key_field).and_then(Value::as_str) == Some(key));
    if record.is_runtime_removed {
        return remove_prompt_cache_runtime_record(
            conversations,
            conversation_index,
            record,
            &occurred_at,
        );
    }
    let Some(preview) = record.preview.as_ref() else {
        return Ok(false);
    };
    let preview = serde_json::to_value(preview)?;
    let index = conversation_index.unwrap_or_else(|| {
        conversations.push(new_prompt_cache_conversation(
            topic,
            key_field,
            key,
            &occurred_at,
        ));
        conversations.len() - 1
    });
    let Some(conversation) = conversations[index].as_object_mut() else {
        return Ok(false);
    };
    apply_prompt_cache_preview(topic, conversation, &occurred_at, preview);
    apply_prompt_cache_terminal_delta(
        topic,
        conversation,
        record,
        &occurred_at,
        applied_terminal_ids,
        baseline_row_id,
    );
    Ok(true)
}

fn prompt_cache_record_target<'a>(
    topic: &SubscriptionTopic,
    record: &'a PromptCacheTopicDelta,
) -> Option<(&'static str, &'a str)> {
    match topic {
        SubscriptionTopic::PromptCacheWindow { .. } => record
            .prompt_cache_key
            .as_deref()
            .map(|key| ("promptCacheKey", key)),
        SubscriptionTopic::PromptCacheStickyWindow { account_id, .. }
            if record.upstream_account_id == Some(*account_id) =>
        {
            record.sticky_key.as_deref().map(|key| ("stickyKey", key))
        }
        _ => None,
    }
}

fn remove_prompt_cache_runtime_record(
    conversations: &mut [Value],
    conversation_index: Option<usize>,
    record: &PromptCacheTopicDelta,
    occurred_at: &Value,
) -> Result<bool, ApiError> {
    let Some(index) = conversation_index else {
        return Ok(false);
    };
    let Some(conversation) = conversations[index].as_object_mut() else {
        return Ok(false);
    };
    let Some(recent) = conversation
        .get_mut("recentInvocations")
        .and_then(Value::as_array_mut)
    else {
        return Ok(false);
    };
    let count_before = recent.len();
    recent.retain(|item| {
        item.get("invokeId").and_then(Value::as_str) != Some(record.invoke_id.as_str())
            || item.get("occurredAt") != Some(occurred_at)
    });
    Ok(recent.len() != count_before)
}

fn new_prompt_cache_conversation(
    topic: &SubscriptionTopic,
    key_field: &str,
    key: &str,
    occurred_at: &Value,
) -> Value {
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
    Value::Object(conversation)
}

fn apply_prompt_cache_preview(
    topic: &SubscriptionTopic,
    conversation: &mut serde_json::Map<String, Value>,
    occurred_at: &Value,
    preview: Value,
) {
    let recent = conversation
        .entry("recentInvocations")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("prompt cache recentInvocations is an array");
    let existing_index = recent.iter().position(|item| {
        item.get("invokeId").and_then(Value::as_str)
            == preview.get("invokeId").and_then(Value::as_str)
            && item.get("occurredAt") == Some(occurred_at)
    });
    if let Some(index) = existing_index {
        recent[index] = preview;
    } else {
        recent.push(preview);
    }
    recent.sort_by(|left, right| {
        right
            .get("occurredAt")
            .and_then(Value::as_str)
            .cmp(&left.get("occurredAt").and_then(Value::as_str))
    });
    recent.truncate(prompt_cache_recent_limit(topic));
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
}

fn prompt_cache_recent_limit(topic: &SubscriptionTopic) -> usize {
    match topic {
        SubscriptionTopic::PromptCacheWindow {
            recent_invocation_limit,
            ..
        } => recent_invocation_limit.unwrap_or(16).max(0) as usize,
        SubscriptionTopic::PromptCacheStickyWindow { .. } => 5,
        _ => 0,
    }
}

fn apply_prompt_cache_terminal_delta(
    topic: &SubscriptionTopic,
    conversation: &mut serde_json::Map<String, Value>,
    record: &PromptCacheTopicDelta,
    occurred_at: &Value,
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) {
    let already_terminal = applied_terminal_ids.contains(&record.identity)
        || (record.row_id > 0 && record.row_id <= baseline_row_id);
    if !record.is_terminal || already_terminal {
        return;
    }
    applied_terminal_ids.insert(record.identity.clone());
    increment_json_i64(conversation, "requestCount", 1);
    increment_json_i64(conversation, "totalTokens", record.request_tokens);
    increment_json_f64(conversation, "totalCost", record.cost);
    if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. }) {
        if conversation
            .get("lastTerminalAt")
            .and_then(Value::as_str)
            .is_none_or(|current| occurred_at.as_str().is_some_and(|next| next > current))
        {
            conversation.insert("lastTerminalAt".to_string(), occurred_at.clone());
        }
        apply_prompt_cache_account_delta(conversation, record, occurred_at);
    }
    append_prompt_cache_terminal_point(topic, conversation, record, occurred_at);
}

fn append_prompt_cache_terminal_point(
    topic: &SubscriptionTopic,
    conversation: &mut serde_json::Map<String, Value>,
    record: &PromptCacheTopicDelta,
    occurred_at: &Value,
) {
    let points = conversation
        .entry("last24hRequests")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .expect("prompt cache last24hRequests is an array");
    if points.iter().any(|point| {
        point.get("occurredAt") == Some(occurred_at)
            && point.get("requestTokens").and_then(Value::as_i64) == Some(record.request_tokens)
    }) {
        return;
    }
    let mut point = serde_json::json!({
        "occurredAt": occurred_at,
        "status": record.status,
        "isSuccess": record.is_success,
        "requestTokens": record.request_tokens,
        "cumulativeTokens": 0,
    });
    if matches!(topic, SubscriptionTopic::PromptCacheWindow { .. })
        && let Some(point) = point.as_object_mut()
    {
        point.insert(
            "outcome".to_string(),
            Value::String(
                if record.is_success {
                    "success"
                } else {
                    "failure"
                }
                .to_string(),
            ),
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

fn increment_json_i64(object: &mut serde_json::Map<String, Value>, field: &str, delta: i64) {
    let current = object
        .get(field)
        .and_then(Value::as_i64)
        .unwrap_or_default();
    object.insert(
        field.to_string(),
        Value::from(current.saturating_add(delta)),
    );
}

fn prompt_cache_delta_needs_replay(
    delta: &PromptCacheTopicDelta,
    persisted_identities: &HashSet<String>,
) -> bool {
    !persisted_identities.contains(&delta.identity)
}

fn increment_json_f64(object: &mut serde_json::Map<String, Value>, field: &str, delta: f64) {
    let current = object
        .get(field)
        .and_then(Value::as_f64)
        .unwrap_or_default();
    object.insert(field.to_string(), Value::from(current + delta));
}

fn apply_prompt_cache_account_delta(
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

fn set_json_optional_field(
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

fn dashboard_activity_payload_exact_range(payload: &Value) -> Option<ExactUtcRange> {
    let object = payload.as_object()?;
    let range_start = object.get("rangeStart")?.as_str()?;
    let range_end = object.get("rangeEnd")?.as_str()?;
    Some(ExactUtcRange {
        start: parse_to_utc_datetime(range_start)?,
        end: parse_to_utc_datetime(range_end)?,
    })
}

async fn dashboard_activity_snapshot_selection_for_topic(
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

fn dashboard_activity_account_sort_tuple(value: &Value) -> (i64, Option<&str>, i64) {
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

fn apply_dashboard_activity_account_overlays(
    accounts: &mut [Value],
    live: &DashboardActivityLiveSnapshot,
    current_snapshot_by_account: &HashMap<Option<i64>, DashboardActivityCurrentSnapshot>,
) -> Result<HashSet<String>, ApiError> {
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
        apply_dashboard_activity_account_latency_fields(account_object, current_snapshot);
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
    Ok(existing_account_keys)
}

fn apply_dashboard_activity_account_latency_fields(
    account: &mut serde_json::Map<String, Value>,
    snapshot: DashboardActivityCurrentSnapshot,
) {
    set_json_optional_field(
        account,
        "currentFirstResponseByteTotalAvgMs",
        snapshot
            .first_response_byte_total_avg_ms()
            .map(|value| json!(value)),
    );
    set_json_optional_field(
        account,
        "currentFirstTokenAvgMs",
        snapshot.first_token_avg_ms().map(|value| json!(value)),
    );
    set_json_optional_field(
        account,
        "currentAvgTotalMs",
        snapshot.avg_total_ms().map(|value| json!(value)),
    );
    set_json_optional_field(
        account,
        "currentAvgResponseMs",
        snapshot
            .avg_response_duration_ms()
            .map(|value| json!(value)),
    );
}

fn apply_dashboard_activity_live_overlay_to_payload(
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
    if !apply_dashboard_activity_summary_live_overlay(root, live, current_snapshot_summary)? {
        return Ok(false);
    }

    let Some(accounts_value) = root.get_mut("accounts") else {
        return Ok(true);
    };
    let Some(accounts) = accounts_value.as_array_mut() else {
        return Ok(true);
    };

    let existing_account_keys =
        apply_dashboard_activity_account_overlays(accounts, live, &current_snapshot_by_account)?;

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

fn apply_dashboard_activity_summary_live_overlay(
    root: &mut serde_json::Map<String, Value>,
    live: &DashboardActivityLiveSnapshot,
    current_snapshot_summary: DashboardActivityCurrentSnapshot,
) -> Result<bool, ApiError> {
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
    Ok(true)
}

fn dashboard_activity_response_exact_range(
    response: &DashboardActivityResponse,
) -> Option<ExactUtcRange> {
    Some(ExactUtcRange {
        start: parse_to_utc_datetime(&response.range_start)?,
        end: parse_to_utc_datetime(&response.range_end)?,
    })
}
