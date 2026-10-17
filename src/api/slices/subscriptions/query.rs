use super::*;

pub(crate) fn decode_topics_query(
    raw: Option<&str>,
) -> Result<Vec<SubscriptionTopicDescriptor>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    decode_query_json(raw, "topics")
}

pub(crate) fn decode_resume_query(
    raw: Option<&str>,
    descriptors: &[SubscriptionTopicDescriptor],
) -> Result<Vec<SubscriptionResumeCursor>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let query_items = decode_query_json::<Vec<SubscriptionResumeCursorQuery>>(raw, "resume")?;
    query_items
        .into_iter()
        .map(|item| match item {
            SubscriptionResumeCursorQuery::Legacy(cursor) => Ok(cursor),
            SubscriptionResumeCursorQuery::Compact(cursor) => {
                let descriptor = descriptors.get(cursor.topic_index).ok_or_else(|| {
                    ApiError::bad_request(anyhow!(
                        "resume topicIndex out of range: {}",
                        cursor.topic_index
                    ))
                })?;
                let topic_key = SubscriptionTopic::from_descriptor(descriptor)?.cache_key()?;
                Ok(SubscriptionResumeCursor {
                    topic_key,
                    cursor: cursor.cursor,
                    schema_epoch: cursor.schema_epoch,
                })
            }
        })
        .collect()
}

pub(crate) async fn next_task_observation_change(
    receiver: &mut Option<
        tokio::sync::broadcast::Receiver<crate::task_timeline::TaskObservationChange>,
    >,
) -> Result<crate::task_timeline::TaskObservationChange, tokio::sync::broadcast::error::RecvError> {
    match receiver {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

pub(crate) fn managed_task_change_matches_topic(
    change: &crate::task_timeline::TaskObservationChange,
    topic: &SubscriptionTopic,
) -> bool {
    match change {
        crate::task_timeline::TaskObservationChange::Runtime => {
            matches!(
                topic,
                SubscriptionTopic::ManagedTaskCatalog | SubscriptionTopic::ManagedTaskRuntime
            )
        }
        crate::task_timeline::TaskObservationChange::Timeline => {
            matches!(
                topic,
                SubscriptionTopic::ManagedTaskCatalog
                    | SubscriptionTopic::ManagedTaskTimeline
                    | SubscriptionTopic::ManagedTaskTimelineV2
            )
        }
        crate::task_timeline::TaskObservationChange::Workload(task_key) => match topic {
            SubscriptionTopic::ManagedTaskCatalog => true,
            SubscriptionTopic::ManagedTaskDetail {
                task_key: subscribed_task_key,
            }
            | SubscriptionTopic::ManagedTaskWorkload {
                task_key: subscribed_task_key,
                ..
            } => subscribed_task_key == task_key,
            _ => false,
        },
    }
}

pub(crate) fn decode_query_json<T: DeserializeOwned>(
    raw: &str,
    field: &str,
) -> Result<T, ApiError> {
    if raw.starts_with('[') || raw.starts_with('{') {
        return serde_json::from_str(raw).map_err(ApiError::from);
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(raw)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(raw))
        .map_err(|err| ApiError::bad_request(anyhow!("invalid {field} payload: {err}")))?;
    serde_json::from_slice(&bytes).map_err(ApiError::from)
}

pub(crate) fn serialize_topic_frame(
    descriptor: SubscriptionTopicDescriptor,
    topic_key: String,
    schema_epoch: String,
    cursor: u64,
    payload_bytes: Vec<u8>,
) -> Result<SerializedTopicFrame, ApiError> {
    let mut hasher = DefaultHasher::new();
    payload_bytes.hash(&mut hasher);
    let fingerprint = hasher.finish();
    let descriptor_json = serde_json::to_string(&descriptor)?;
    let topic_key_json = serde_json::to_string(&topic_key)?;
    let schema_epoch_json = serde_json::to_string(&schema_epoch)?;
    let envelope_metadata_bytes = Bytes::from(format!(
        r#"","topic":{descriptor_json},"topicKey":{topic_key_json},"schemaEpoch":{schema_epoch_json},"cursor":{cursor},"payload":"#
    ));
    Ok(SerializedTopicFrame {
        topic_key,
        schema_epoch,
        cursor,
        descriptor,
        fingerprint,
        payload_bytes: Bytes::from(payload_bytes),
        envelope_metadata_bytes,
    })
}

pub(crate) fn serialize_topic_delta_frame(
    descriptor: SubscriptionTopicDescriptor,
    topic_key: String,
    schema_epoch: String,
    cursor: u64,
    base_cursor: u64,
    payload_bytes: Vec<u8>,
) -> Result<SerializedTopicFrame, ApiError> {
    let mut hasher = DefaultHasher::new();
    payload_bytes.hash(&mut hasher);
    let fingerprint = hasher.finish();
    let descriptor_json = serde_json::to_string(&descriptor)?;
    let topic_key_json = serde_json::to_string(&topic_key)?;
    let schema_epoch_json = serde_json::to_string(&schema_epoch)?;
    let envelope_metadata_bytes = Bytes::from(format!(
        r#"","topic":{descriptor_json},"topicKey":{topic_key_json},"schemaEpoch":{schema_epoch_json},"cursor":{cursor},"delivery":"delta","baseCursor":{base_cursor},"payload":"#
    ));
    Ok(SerializedTopicFrame {
        topic_key,
        schema_epoch,
        cursor,
        descriptor,
        fingerprint,
        payload_bytes: Bytes::from(payload_bytes),
        envelope_metadata_bytes,
    })
}

pub(crate) fn build_timeseries_delta_payload(
    previous_bytes: &[u8],
    next_bytes: &[u8],
) -> Option<Vec<u8>> {
    let previous = serde_json::from_slice::<Value>(previous_bytes).ok()?;
    let next = serde_json::from_slice::<Value>(next_bytes).ok()?;
    let previous_object = previous.as_object()?;
    let next_object = next.as_object()?;
    let previous_points = previous_object.get("points")?.as_array()?;
    let next_points = next_object.get("points")?.as_array()?;
    if previous_points.len() != next_points.len() {
        return None;
    }

    for (key, previous_value) in previous_object {
        if matches!(key.as_str(), "rangeEnd" | "snapshotId" | "points") {
            continue;
        }
        if next_object.get(key) != Some(previous_value) {
            return None;
        }
    }
    for (key, next_value) in next_object {
        if matches!(key.as_str(), "rangeEnd" | "snapshotId" | "points") {
            continue;
        }
        if previous_object.get(key) != Some(next_value) {
            return None;
        }
    }

    let changed_points = previous_points
        .iter()
        .zip(next_points)
        .enumerate()
        .filter_map(|(index, (previous, next))| (previous != next).then_some((index, next)))
        .collect::<Vec<_>>();
    if changed_points.len() > 1 {
        return None;
    }
    let (point_index, point) = changed_points
        .first()
        .map(|(index, point)| (Value::from(*index), (*point).clone()))
        .unwrap_or((Value::Null, Value::Null));
    serde_json::to_vec(&serde_json::json!({
        "kind": "timeseries-point/v1",
        "rangeEnd": next_object.get("rangeEnd")?,
        "snapshotId": next_object.get("snapshotId")?,
        "pointIndex": point_index,
        "point": point,
    }))
    .ok()
}

pub(crate) fn build_dashboard_delta_payload(
    topic: &SubscriptionTopic,
    previous_bytes: &[u8],
    next_bytes: &[u8],
) -> Option<Vec<u8>> {
    if topic.uses_timeseries_live_projection() {
        return build_timeseries_delta_payload(previous_bytes, next_bytes);
    }
    if matches!(
        topic,
        SubscriptionTopic::DashboardWorkingConversationsCurrent { .. }
    ) {
        return build_collection_item_delta_payload(previous_bytes, next_bytes, "conversations");
    }
    None
}

fn build_collection_item_delta_payload(
    previous_bytes: &[u8],
    next_bytes: &[u8],
    collection: &str,
) -> Option<Vec<u8>> {
    let previous = serde_json::from_slice::<Value>(previous_bytes).ok()?;
    let next = serde_json::from_slice::<Value>(next_bytes).ok()?;
    let previous_object = previous.as_object()?;
    let next_object = next.as_object()?;
    let previous_items = previous_object.get(collection)?.as_array()?;
    let next_items = next_object.get(collection)?.as_array()?;
    if previous_items.len() != next_items.len() {
        return None;
    }
    let mut changed_fields = serde_json::Map::new();
    for (key, previous_value) in previous_object {
        if key == collection {
            continue;
        }
        let next_value = next_object.get(key)?;
        if next_value != previous_value {
            changed_fields.insert(key.clone(), next_value.clone());
        }
    }
    if next_object
        .keys()
        .any(|key| key != collection && !previous_object.contains_key(key))
    {
        return None;
    }
    let changed_items = previous_items
        .iter()
        .zip(next_items)
        .enumerate()
        .filter_map(|(index, (previous, next))| (previous != next).then_some((index, next)))
        .collect::<Vec<_>>();
    if changed_items.len() > 1 {
        return None;
    }
    let (item_index, item) = changed_items
        .first()
        .map(|(index, item)| (Value::from(*index), (*item).clone()))
        .unwrap_or((Value::Null, Value::Null));
    if changed_fields.is_empty() && item.is_null() {
        return None;
    }
    serde_json::to_vec(&serde_json::json!({
        "kind": "collection-item/v1",
        "collection": collection,
        "itemIndex": item_index,
        "item": item,
        "fields": Value::Object(changed_fields),
    }))
    .ok()
}

pub(crate) fn serialize_unavailable_topic_frame(
    descriptor: SubscriptionTopicDescriptor,
    topic_key: String,
    schema_epoch: String,
) -> Result<SerializedTopicFrame, ApiError> {
    let descriptor_json = serde_json::to_string(&descriptor)?;
    let topic_key_json = serde_json::to_string(&topic_key)?;
    let schema_epoch_json = serde_json::to_string(&schema_epoch)?;
    let envelope_metadata_bytes = Bytes::from(format!(
        r#"","topic":{descriptor_json},"topicKey":{topic_key_json},"schemaEpoch":{schema_epoch_json},"errorCode":"unavailable""#
    ));
    Ok(SerializedTopicFrame {
        topic_key,
        schema_epoch,
        cursor: 0,
        descriptor,
        fingerprint: 0,
        payload_bytes: Bytes::new(),
        envelope_metadata_bytes,
    })
}

pub(crate) fn reuse_unchanged_cached_topic(
    existing: &mut CachedSubscriptionTopic,
    serialized_payload: &[u8],
) -> Option<CachedSubscriptionTopic> {
    if existing.snapshot_frame.payload_bytes.as_ref() != serialized_payload {
        return None;
    }
    existing.dirty = false;
    existing.refresh_scheduled = false;
    existing.runtime_topic_recovery_retry_at = None;
    // A successful Prompt Cache baseline can serialize identically to last-good. It still
    // proves reconciliation completed, so do not keep scheduling the same cold hydration.
    existing.prompt_cache_reconcile_required = false;
    existing.prompt_cache_pressure_deferred = false;
    existing.prompt_cache_pending_key_hydrations.clear();
    existing.prompt_cache_candidate_refill_required = false;
    existing.prompt_cache_key_hydration_scheduled = false;
    existing.snapshot_built_at = Instant::now();
    Some(existing.clone())
}

pub(crate) fn finish_prompt_cache_baseline_reuse(
    existing: &mut CachedSubscriptionTopic,
    build: &PromptCacheBaselineBuild,
    applied_terminal_ids: &HashSet<String>,
) {
    existing.prompt_cache_full_hydration_count =
        existing.prompt_cache_full_hydration_count.saturating_add(1);
    existing.prompt_cache_baseline_at = Some(Instant::now());
    existing.prompt_cache_baseline_row_id = build.baseline_row_id;
    existing.prompt_cache_response_source = "database_reconcile";
    existing.prompt_cache_applied_terminal_ids = applied_terminal_ids.clone();
}

pub(crate) fn observe_prompt_cache_baseline_publication(metrics: &ObservabilityRuntime) {
    use crate::observability::contention::{PerformanceGauge, StatsProjection};
    metrics.set_performance(
        PerformanceGauge::ProjectionLastGood(StatsProjection::PromptCache),
        Utc::now().timestamp_millis() as f64 / 1_000.0,
    );
}

pub(crate) fn prune_replay_window(
    events: &mut VecDeque<ReplayableTopicEvent>,
    total_bytes: &mut usize,
) {
    let cutoff = Utc::now() - ChronoDuration::seconds(SUBSCRIPTION_REPLAY_WINDOW_SECS);
    while let Some(front) = events.front() {
        let should_drop = events.len() > SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC
            || *total_bytes > SUBSCRIPTION_REPLAY_MAX_BYTES_PER_TOPIC
            || front.emitted_at < cutoff;
        if !should_drop {
            break;
        }
        if let Some(removed) = events.pop_front() {
            *total_bytes = total_bytes.saturating_sub(removed.bytes);
        }
    }
}

pub(crate) fn btree_map_from_pairs<const N: usize>(
    pairs: [(&str, String); N],
) -> BTreeMap<String, String> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

pub(crate) fn insert_optional_param(
    params: &mut BTreeMap<String, String>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        params.insert(key.to_string(), value);
    }
}

pub(crate) fn param_or_default(
    params: &BTreeMap<String, String>,
    key: &str,
    default: &str,
) -> String {
    params
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub(crate) fn parse_optional_text_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Option<String> {
    params
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn parse_required_text_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<String, ApiError> {
    parse_optional_text_param(params, key).ok_or_else(|| {
        ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
    })
}

pub(crate) fn parse_required_i64_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<i64, ApiError> {
    parse_optional_i64_param(params, key)?.ok_or_else(|| {
        ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
    })
}

pub(crate) fn parse_required_positive_i64_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<i64, ApiError> {
    let value = parse_required_i64_param(params, key)?;
    if value > 0 {
        return Ok(value);
    }
    Err(ApiError::bad_request(anyhow!(
        "subscription topic param `{key}` must be a positive integer"
    )))
}

pub(crate) fn parse_upstream_account_attempt_page_param(
    params: &BTreeMap<String, String>,
    key: &str,
    default: usize,
) -> Result<usize, ApiError> {
    let value = parse_optional_i64_param(params, key)?;
    let value = match value {
        Some(value) if value >= 0 => Some(
            usize::try_from(value)
                .map_err(|_| ApiError::bad_request(anyhow!("invalid integer for `{key}`")))?,
        ),
        Some(_) => {
            return Err(ApiError::bad_request(anyhow!(
                "subscription topic param `{key}` must not be negative"
            )));
        }
        None => None,
    };
    let value = value.unwrap_or(default);
    Ok(if key == "page" {
        normalize_upstream_account_list_page(Some(value))
    } else {
        normalize_upstream_account_list_page_size(Some(value))
    })
}

pub(crate) fn normalize_upstream_account_attempt_type_param(
    params: &BTreeMap<String, String>,
) -> Option<String> {
    parse_optional_text_param(params, "type")
        .map(|value| value.to_ascii_lowercase())
        .filter(|value| matches!(value.as_str(), "normal" | "remote_v2" | "image" | "compact"))
}

pub(crate) fn parse_i64_param(
    params: &BTreeMap<String, String>,
    key: &str,
    default: Option<i64>,
) -> Result<i64, ApiError> {
    parse_optional_i64_param(params, key)?
        .or(default)
        .ok_or_else(|| {
            ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
        })
}

pub(crate) fn parse_optional_i64_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<i64>, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return Ok(None);
    };
    value
        .parse::<i64>()
        .map(Some)
        .map_err(|err| ApiError::bad_request(anyhow!("invalid integer for `{key}`: {err}")))
}

pub(crate) fn parse_optional_blocked_binding_constraint_source_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<BlockedBindingConstraintSource>, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return Ok(None);
    };
    BlockedBindingConstraintSource::from_query_param(&value)
        .ok_or_else(|| {
            ApiError::bad_request(anyhow!(
                "{key} must be one of: upstreamAccountBinding, encryptedSessionOwner"
            ))
        })
        .map(Some)
}

pub(crate) fn parse_optional_u8_param(
    params: &BTreeMap<String, String>,
    key: &str,
) -> Result<Option<u8>, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return Ok(None);
    };
    value
        .parse::<u8>()
        .map(Some)
        .map_err(|err| ApiError::bad_request(anyhow!("invalid integer for `{key}`: {err}")))
}

pub(crate) fn parse_bool_param(
    params: &BTreeMap<String, String>,
    key: &str,
    default: Option<bool>,
) -> Result<bool, ApiError> {
    let Some(value) = parse_optional_text_param(params, key) else {
        return default.ok_or_else(|| {
            ApiError::bad_request(anyhow!("subscription topic param `{key}` is required"))
        });
    };
    match value.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ApiError::bad_request(anyhow!(
            "invalid boolean for `{key}`: {value}"
        ))),
    }
}

pub(crate) fn parse_conversation_subscription_scope(
    params: &BTreeMap<String, String>,
) -> Result<ConversationSubscriptionScope, ApiError> {
    let prompt_cache_key = parse_optional_text_param(params, "promptCacheKey");
    let sticky_key = parse_optional_text_param(params, "stickyKey");
    match (prompt_cache_key, sticky_key) {
        (Some(prompt_cache_key), None) => Ok(ConversationSubscriptionScope::PromptCacheKey(
            prompt_cache_key,
        )),
        (None, Some(sticky_key)) => {
            let upstream_account_id = parse_required_i64_param(params, "upstreamAccountId")?;
            if upstream_account_id <= 0 {
                return Err(ApiError::bad_request(anyhow!(
                    "upstreamAccountId must be positive for stickyKey subscription scope"
                )));
            }
            Ok(ConversationSubscriptionScope::StickyKey {
                sticky_key,
                upstream_account_id,
            })
        }
        (Some(_), Some(_)) => Err(ApiError::bad_request(anyhow!(
            "promptCacheKey and stickyKey are mutually exclusive subscription scope params"
        ))),
        (None, None) => Err(ApiError::bad_request(anyhow!(
            "promptCacheKey or stickyKey + upstreamAccountId is required for conversation subscription"
        ))),
    }
}

pub(crate) fn parse_optional_conversation_operation_info_type(
    params: &BTreeMap<String, String>,
) -> Result<Option<String>, ApiError> {
    let Some(info_type) = parse_optional_text_param(params, "infoType") else {
        return Ok(None);
    };
    match info_type.as_str() {
        PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_ROUTING
        | PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_FORWARD_PROXY
        | PROMPT_CACHE_CONVERSATION_OPERATION_INFO_TYPE_REQUEST_REWRITE => Ok(Some(info_type)),
        _ => Err(ApiError::bad_request(anyhow!(
            "infoType must be one of: routing, forwardProxy, requestRewrite"
        ))),
    }
}

pub(crate) fn parse_prompt_cache_selection(
    params: &BTreeMap<String, String>,
) -> Result<PromptCacheConversationSelection, ApiError> {
    let limit = parse_optional_i64_param(params, "limit")?;
    let activity_hours = parse_optional_i64_param(params, "activityHours")?;
    let activity_minutes = parse_optional_i64_param(params, "activityMinutes")?;
    resolve_prompt_cache_conversation_selection(PromptCacheConversationsQuery {
        limit,
        activity_hours,
        activity_minutes,
        page_size: None,
        cursor: None,
        snapshot_at: None,
        detail: None,
        recent_invocation_limit: None,
        blocked_binding_upstream_account_id: None,
        blocked_binding_constraint_source: None,
    })
}

pub(crate) fn parse_prompt_cache_detail_level(
    params: &BTreeMap<String, String>,
) -> Result<PromptCacheConversationDetailLevel, ApiError> {
    resolve_prompt_cache_conversation_detail_level(
        parse_optional_text_param(params, "detail").as_deref(),
    )
}

pub(crate) fn parse_sticky_selection(
    params: &BTreeMap<String, String>,
) -> Result<AccountStickyKeySelection, ApiError> {
    resolve_sticky_key_selection(&AccountStickyKeysQuery {
        limit: parse_optional_i64_param(params, "limit")?,
        activity_hours: parse_optional_i64_param(params, "activityHours")?,
    })
    .map_err(|(_, message)| ApiError::bad_request(anyhow!(message)))
}

pub(crate) fn prompt_cache_selection_params(
    selection: PromptCacheConversationSelection,
) -> BTreeMap<String, String> {
    match selection {
        PromptCacheConversationSelection::Count(limit) => {
            BTreeMap::from([("limit".to_string(), limit.to_string())])
        }
        PromptCacheConversationSelection::ActivityWindowHours(hours) => {
            BTreeMap::from([("activityHours".to_string(), hours.to_string())])
        }
        PromptCacheConversationSelection::ActivityWindowMinutes(minutes) => {
            BTreeMap::from([("activityMinutes".to_string(), minutes.to_string())])
        }
    }
}
