pub(crate) fn prompt_cache_runtime_record_source_matches(
    record: &ApiInvocation,
    source_scope: InvocationSourceScope,
) -> bool {
    source_scope == InvocationSourceScope::All || record.source == SOURCE_PROXY
}

pub(crate) fn prompt_cache_runtime_record_is_in_flight(record: &ApiInvocation) -> bool {
    matches!(
        record
            .status
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "running" | "pending"
    )
}

pub(crate) fn prompt_cache_runtime_record_is_in_working_window(
    record: &ApiInvocation,
    range_start_bound: &str,
) -> bool {
    prompt_cache_runtime_record_is_in_flight(record)
        || parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
            db_occurred_at_lower_bound(occurred_at).as_str() >= range_start_bound
        })
}

pub(crate) fn prompt_cache_runtime_record_matches_blocked_binding_filter(
    record: &ApiInvocation,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) -> bool {
    let Some(blocked_binding_filter) = blocked_binding_filter else {
        return true;
    };
    if !blocked_binding_filter.is_active() {
        return true;
    }
    let Some(blocked_binding) = record.blocked_binding.as_ref() else {
        return false;
    };
    blocked_binding_filter
        .upstream_account_id
        .is_none_or(|value| blocked_binding.upstream_account_id == value)
        && blocked_binding_filter
            .constraint_source
            .is_none_or(|value| blocked_binding.constraint_source == value)
}

pub(crate) fn prompt_cache_runtime_record_sort_anchor(record: &ApiInvocation) -> String {
    record.occurred_at.clone()
}

pub(crate) fn max_optional_timestamp(
    left: Option<String>,
    right: Option<String>,
) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(crate) fn merge_runtime_prompt_cache_aggregate(
    aggregate: &mut PromptCacheConversationAggregateRow,
    runtime: PromptCacheConversationAggregateRow,
) {
    aggregate.request_count += runtime.request_count;
    aggregate.total_tokens += runtime.total_tokens;
    aggregate.total_cost += runtime.total_cost;
    if runtime.created_at < aggregate.created_at {
        aggregate.created_at = runtime.created_at;
    }
    if runtime.last_activity_at > aggregate.last_activity_at {
        aggregate.last_activity_at = runtime.last_activity_at;
    }
    aggregate.sort_anchor_at =
        max_optional_timestamp(aggregate.sort_anchor_at.take(), runtime.sort_anchor_at);
    aggregate.last_terminal_at =
        max_optional_timestamp(aggregate.last_terminal_at.take(), runtime.last_terminal_at);
    aggregate.last_in_flight_at = max_optional_timestamp(
        aggregate.last_in_flight_at.take(),
        runtime.last_in_flight_at,
    );
}

pub(crate) fn runtime_prompt_cache_aggregate_from_record(
    record: &ApiInvocation,
) -> Option<PromptCacheConversationAggregateRow> {
    let prompt_cache_key = record.prompt_cache_key.as_deref()?.trim();
    if prompt_cache_key.is_empty() {
        return None;
    }
    let occurred_at = record.occurred_at.clone();
    let is_in_flight = prompt_cache_runtime_record_is_in_flight(record);
    Some(PromptCacheConversationAggregateRow {
        prompt_cache_key: prompt_cache_key.to_string(),
        request_count: 1,
        total_tokens: record.total_tokens.unwrap_or_default().max(0),
        total_cost: record.cost.unwrap_or_default(),
        created_at: occurred_at.clone(),
        last_activity_at: occurred_at.clone(),
        cursor_created_at: Some(occurred_at.clone()),
        sort_anchor_at: Some(prompt_cache_runtime_record_sort_anchor(record)),
        last_terminal_at: (!is_in_flight).then(|| occurred_at.clone()),
        last_in_flight_at: is_in_flight.then_some(occurred_at),
    })
}

pub(crate) fn runtime_prompt_cache_overlay_records(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range_start_bound: &str,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
) -> Vec<ApiInvocation> {
    state
        .proxy_runtime_invocations
        .snapshot()
        .into_iter()
        .filter(|record| prompt_cache_runtime_record_source_matches(record, source_scope))
        .filter(|record| {
            prompt_cache_runtime_record_is_in_working_window(record, range_start_bound)
        })
        .filter(|record| {
            record
                .prompt_cache_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty())
        })
        .filter(|record| {
            prompt_cache_runtime_record_matches_blocked_binding_filter(
                record,
                blocked_binding_filter,
            )
        })
        .collect()
}

pub(crate) fn runtime_prompt_cache_overlay_records_at_snapshot(
    state: &AppState,
    source_scope: InvocationSourceScope,
    range_start_bound: &str,
    blocked_binding_filter: Option<&PromptCacheConversationBlockedBindingFilter>,
    snapshot_at: DateTime<Utc>,
) -> Vec<ApiInvocation> {
    // Runtime overlay membership follows the working-window event-time contract. Filtering on
    // `created_at` is incorrect because it changes as the runtime record advances phases.
    runtime_prompt_cache_overlay_records(
        state,
        source_scope,
        range_start_bound,
        blocked_binding_filter,
    )
    .into_iter()
    .filter(|record| {
        parse_to_utc_datetime(&record.occurred_at)
            .is_some_and(|occurred_at| occurred_at <= snapshot_at)
    })
    .collect()
}

pub(crate) fn runtime_prompt_cache_overlay_keys(
    runtime_overlay_records: &[ApiInvocation],
) -> HashSet<String> {
    runtime_overlay_records
        .iter()
        .filter_map(|record| {
            record
                .prompt_cache_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
        })
        .collect()
}

fn runtime_prompt_cache_overlay_identity(record: &ApiInvocation) -> String {
    format!("{}\0{}", record.invoke_id, record.occurred_at)
}

async fn transient_runtime_prompt_cache_overlay_records_on_connection(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    identity_visibility_snapshot: Option<&PromptCacheConversationSnapshotFilter>,
    runtime_overlay_records: &[ApiInvocation],
) -> Result<Vec<ApiInvocation>, ApiError> {
    let terminal_records = runtime_overlay_records
        .iter()
        .filter(|record| !prompt_cache_runtime_record_is_in_flight(record))
        .collect::<Vec<_>>();
    if terminal_records.is_empty() {
        return Ok(runtime_overlay_records.to_vec());
    }

    // Runtime entries retain id 0 until the derived-write acknowledgement arrives. A terminal
    // row may already be visible in the durable source that owns this aggregate. Bounded key
    // hydration uses the current working-set aggregate, while paginated snapshots use their
    // pinned visibility boundary; each caller supplies the matching identity visibility scope.
    let mut persisted_terminal_identities = HashSet::new();
    for records in terminal_records.chunks(300) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT invoke_id, occurred_at FROM codex_invocations WHERE ",
        );
        if let Some(snapshot) = identity_visibility_snapshot {
            push_snapshot_invocation_visibility_clause(
                &mut query,
                "occurred_at",
                "id",
                "created_at",
                Some(snapshot),
            );
        } else {
            query.push("1 = 1");
        }
        if source_scope == InvocationSourceScope::ProxyOnly {
            query.push(" AND source = ").push_bind(SOURCE_PROXY);
        }
        query.push(" AND (");
        for (index, record) in records.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(&record.invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(&record.occurred_at)
                .push(")");
        }
        query.push(")");
        for (invoke_id, occurred_at) in query
            .build_query_as::<(String, String)>()
            .fetch_all(&mut *connection)
            .await?
        {
            persisted_terminal_identities.insert(format!("{invoke_id}\0{occurred_at}"));
        }
    }

    Ok(runtime_overlay_records
        .iter()
        .filter(|record| {
            prompt_cache_runtime_record_is_in_flight(record)
                || !persisted_terminal_identities
                    .contains(&runtime_prompt_cache_overlay_identity(record))
        })
        .cloned()
        .collect())
}

pub(crate) fn merge_runtime_prompt_cache_aggregates(
    aggregates: Vec<PromptCacheConversationAggregateRow>,
    runtime_overlay_records: &[ApiInvocation],
    cursor: Option<&(String, String, String, Option<i64>)>,
    limit: i64,
) -> Vec<PromptCacheConversationAggregateRow> {
    if runtime_overlay_records.is_empty() {
        return aggregates;
    }

    let mut rows_by_key = aggregates
        .into_iter()
        .map(|row| (row.prompt_cache_key.clone(), row))
        .collect::<HashMap<_, _>>();
    for record in runtime_overlay_records {
        let Some(runtime) = runtime_prompt_cache_aggregate_from_record(record) else {
            continue;
        };
        if let Some((cursor_sort_anchor_at, cursor_created_at, cursor_prompt_cache_key, _)) = cursor
        {
            let sort_anchor_at = runtime
                .sort_anchor_at
                .as_deref()
                .unwrap_or(&runtime.last_activity_at);
            let cursor_sort_anchor_at = cursor_sort_anchor_at.as_str();
            let cursor_created_at = cursor_created_at.as_str();
            let cursor_prompt_cache_key = cursor_prompt_cache_key.as_str();
            let is_after_cursor = sort_anchor_at < cursor_sort_anchor_at
                || (sort_anchor_at == cursor_sort_anchor_at
                    && (runtime.created_at.as_str() < cursor_created_at
                        || (runtime.created_at.as_str() == cursor_created_at
                            && runtime.prompt_cache_key.as_str() < cursor_prompt_cache_key)));
            if !is_after_cursor {
                continue;
            }
        }
        match rows_by_key.entry(runtime.prompt_cache_key.clone()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_runtime_prompt_cache_aggregate(entry.get_mut(), runtime);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(runtime);
            }
        }
    }

    let mut rows = rows_by_key.into_values().collect::<Vec<_>>();
    sort_prompt_cache_working_aggregates(&mut rows);
    rows.truncate(limit.max(0) as usize);
    rows
}

pub(crate) async fn apply_prompt_cache_lifecycle_aggregate_totals(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    aggregates: &mut [PromptCacheConversationAggregateRow],
    runtime_overlay_records: &[ApiInvocation],
    snapshot_filter: Option<&PromptCacheConversationSnapshotFilter>,
    snapshot_hour_start_epoch: Option<i64>,
    snapshot_hour_start_bound: Option<&str>,
) -> Result<()> {
    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let lifecycle_aggregates = query_prompt_cache_conversation_lifecycle_aggregates(
        pool,
        source_scope,
        &selected_keys,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
    )
    .await?;
    apply_prompt_cache_lifecycle_aggregate_totals_from_lifecycle(
        aggregates,
        runtime_overlay_records,
        lifecycle_aggregates,
    );
    Ok(())
}

pub(crate) async fn apply_prompt_cache_lifecycle_aggregate_totals_on_connection(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
    aggregates: &mut [PromptCacheConversationAggregateRow],
    runtime_overlay_records: &[ApiInvocation],
    snapshot_filter: Option<&PromptCacheConversationSnapshotFilter>,
    snapshot_hour_start_epoch: Option<i64>,
    snapshot_hour_start_bound: Option<&str>,
) -> Result<()> {
    let selected_keys = aggregates
        .iter()
        .map(|row| row.prompt_cache_key.clone())
        .collect::<Vec<_>>();
    let lifecycle_aggregates = query_prompt_cache_conversation_lifecycle_aggregates_on_connection(
        connection,
        source_scope,
        &selected_keys,
        snapshot_filter,
        snapshot_hour_start_epoch,
        snapshot_hour_start_bound,
    )
    .await?;
    apply_prompt_cache_lifecycle_aggregate_totals_from_lifecycle(
        aggregates,
        runtime_overlay_records,
        lifecycle_aggregates,
    );
    Ok(())
}

fn apply_prompt_cache_lifecycle_aggregate_totals_from_lifecycle(
    aggregates: &mut [PromptCacheConversationAggregateRow],
    runtime_overlay_records: &[ApiInvocation],
    lifecycle_aggregates: HashMap<String, PromptCacheConversationAggregateRow>,
) {
    let selected_key_set = aggregates
        .iter()
        .map(|aggregate| aggregate.prompt_cache_key.clone())
        .collect::<HashSet<_>>();
    let mut runtime_aggregates_by_key = HashMap::new();
    for record in runtime_overlay_records {
        let Some(runtime) = runtime_prompt_cache_aggregate_from_record(record) else {
            continue;
        };
        if !selected_key_set.contains(&runtime.prompt_cache_key) {
            continue;
        }
        match runtime_aggregates_by_key.entry(runtime.prompt_cache_key.clone()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                merge_runtime_prompt_cache_aggregate(entry.get_mut(), runtime);
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(runtime);
            }
        }
    }
    for aggregate in aggregates {
        let mut applied_lifecycle = false;
        if let Some(lifecycle) = lifecycle_aggregates.get(&aggregate.prompt_cache_key) {
            aggregate.request_count = lifecycle.request_count;
            aggregate.total_tokens = lifecycle.total_tokens;
            aggregate.total_cost = lifecycle.total_cost;
            aggregate.created_at = lifecycle.created_at.clone();
            aggregate.last_activity_at = lifecycle.last_activity_at.clone();
            applied_lifecycle = true;
        }
        if applied_lifecycle
            && let Some(runtime) = runtime_aggregates_by_key.remove(&aggregate.prompt_cache_key)
        {
            merge_runtime_prompt_cache_aggregate(aggregate, runtime);
        }
    }
}

pub(crate) fn sort_prompt_cache_working_aggregates(
    rows: &mut [PromptCacheConversationAggregateRow],
) {
    rows.sort_by(|left, right| {
        let left_sort = left
            .sort_anchor_at
            .as_deref()
            .unwrap_or(&left.last_activity_at);
        let right_sort = right
            .sort_anchor_at
            .as_deref()
            .unwrap_or(&right.last_activity_at);
        let left_cursor_created = left
            .cursor_created_at
            .as_deref()
            .unwrap_or(&left.created_at);
        let right_cursor_created = right
            .cursor_created_at
            .as_deref()
            .unwrap_or(&right.created_at);
        right_sort
            .cmp(left_sort)
            .then_with(|| right_cursor_created.cmp(left_cursor_created))
            .then_with(|| right.prompt_cache_key.cmp(&left.prompt_cache_key))
    });
}
