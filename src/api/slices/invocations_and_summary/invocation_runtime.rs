fn apply_invocation_scope_filters(
    query: &mut QueryBuilder<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) {
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    if let Some(snapshot_constraint) = snapshot {
        match snapshot_constraint {
            SnapshotConstraint::UpTo(snapshot_id) => {
                query.push(" AND id <= ").push_bind(snapshot_id);
            }
            SnapshotConstraint::After(snapshot_id) => {
                query.push(" AND id > ").push_bind(snapshot_id);
            }
        }
    }

    if let Some(from_bound) = filters.occurred_from.as_ref() {
        query
            .push(" AND occurred_at >= ")
            .push_bind(from_bound.clone());
    }

    if let Some(to_bound) = filters.occurred_to.as_ref() {
        query
            .push(" AND occurred_at < ")
            .push_bind(to_bound.clone());
    }
}

fn apply_invocation_model_filters(
    query: &mut QueryBuilder<Sqlite>,
    filters: &InvocationRecordsFilters,
) {
    if !filters.model_values.is_empty() {
        let target = filters
            .model_target
            .unwrap_or(InvocationModelTarget::Request);
        let model_sql = invocation_model_sql_expr(target);
        push_exact_text_any_filter(query, &model_sql, &filters.model_values);
    } else if let Some(model) = filters.model.as_deref() {
        push_exact_text_filter(query, "model", model);
    }

    if let Some(model_rerouted) = filters.model_rerouted {
        query.push(" AND ");
        match model_rerouted {
            InvocationModelRerouteFilter::Rerouted => {
                query.push(invocation_model_rerouted_sql());
            }
            InvocationModelRerouteFilter::NotRerouted => {
                query.push("NOT (");
                query.push(invocation_model_rerouted_sql());
                query.push(")");
            }
        }
    }
}

fn apply_invocation_status_filter(query: &mut QueryBuilder<Sqlite>, status: Option<&str>) {
    let Some(status) = status else {
        return;
    };
    let normalized_status = status.trim();
    if normalized_status.eq_ignore_ascii_case("failed") {
        // Legacy rows can still represent failures while `status` is NULL/`none`, so align the
        // UI-level failed filter with the same resolved failure-class semantics used by summary.
        query.push(" AND ");
        query.push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL);
        query.push(" IN ('service_failure', 'client_failure', 'client_abort')");
        query.push(" AND ");
        query.push(INVOCATION_STATUS_NORMALIZED_SQL);
        query.push(" != 'interrupted'");
    } else if normalized_status.eq_ignore_ascii_case("success") {
        // Keep the success filter symmetric with the resolved failure-class logic so legacy rows
        // that still carry `status='success'` but classify as failures do not leak into success.
        query.push(" AND ");
        query.push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL);
        query.push(" = 'none'");
        push_exact_text_filter(query, INVOCATION_STATUS_NORMALIZED_SQL, normalized_status);
    } else if normalized_status.eq_ignore_ascii_case(INVOCATION_STATUS_WARNING_SUCCESS) {
        let normalized_status = status.trim();
        query.push(" AND ");
        query.push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL);
        query.push(" = 'none'");
        push_exact_text_filter(query, INVOCATION_STATUS_NORMALIZED_SQL, normalized_status);
    } else {
        push_exact_text_filter(query, "status", status);
    }
}

fn apply_invocation_dimension_filters(
    query: &mut QueryBuilder<Sqlite>,
    filters: &InvocationRecordsFilters,
) {
    if let Some(endpoint) = filters.endpoint.as_deref() {
        push_exact_text_filter(query, INVOCATION_ENDPOINT_SQL, endpoint);
    }

    if let Some(request_id) = filters.request_id.as_deref() {
        push_exact_text_filter(query, "invoke_id", request_id);
    }

    if let Some(failure_class) = filters.failure_class.as_deref() {
        push_exact_text_filter(query, INVOCATION_RESOLVED_FAILURE_CLASS_SQL, failure_class);
    }

    if let Some(failure_kind) = filters.failure_kind.as_deref() {
        push_exact_text_filter(query, INVOCATION_FAILURE_KIND_SQL, failure_kind);
    }

    if let Some(prompt_cache_key) = filters.prompt_cache_key.as_deref() {
        push_exact_text_filter(query, INVOCATION_PROMPT_CACHE_KEY_SQL, prompt_cache_key);
    }

    if let Some(sticky_key) = filters.sticky_key.as_deref() {
        push_exact_text_filter(query, INVOCATION_STICKY_KEY_SQL, sticky_key);
    }

    if let Some(upstream_scope) = filters.upstream_scope.as_deref() {
        push_exact_text_filter(query, INVOCATION_UPSTREAM_SCOPE_SQL, upstream_scope);
    }

    if let Some(upstream_account_id) = filters.upstream_account_id {
        query.push(" AND ").push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL);
        query.push(" = ").push_bind(upstream_account_id);
    }

    if let Some(proxy_display_name) = filters.proxy_display_name.as_deref() {
        push_exact_text_filter(query, INVOCATION_PROXY_DISPLAY_SQL, proxy_display_name);
    }

    if let Some(transport) = filters.transport.as_deref() {
        push_exact_text_filter(query, INVOCATION_TRANSPORT_SQL, transport);
    }

    if let Some(service_tier) = filters.service_tier.as_deref() {
        push_exact_text_filter(query, INVOCATION_SERVICE_TIER_SQL, service_tier);
    }

    if !filters.reasoning_effort_values.is_empty() {
        push_exact_text_any_filter(
            query,
            INVOCATION_REASONING_EFFORT_SQL,
            &filters.reasoning_effort_values,
        );
    } else if let Some(reasoning_effort) = filters.reasoning_effort.as_deref() {
        push_exact_text_filter(query, INVOCATION_REASONING_EFFORT_SQL, reasoning_effort);
    }

    if let Some(requester_ip) = filters.requester_ip.as_deref() {
        push_exact_text_filter(query, INVOCATION_REQUESTER_IP_SQL, requester_ip);
    }

    if let Some(keyword) = filters.keyword.as_deref() {
        push_keyword_filter(query, keyword);
    }
}

fn apply_invocation_numeric_filters(
    query: &mut QueryBuilder<Sqlite>,
    filters: &InvocationRecordsFilters,
) {
    if let Some(min_total_tokens) = filters.min_total_tokens {
        query
            .push(" AND total_tokens IS NOT NULL AND total_tokens >= ")
            .push_bind(min_total_tokens);
    }

    if let Some(max_total_tokens) = filters.max_total_tokens {
        query
            .push(" AND total_tokens IS NOT NULL AND total_tokens <= ")
            .push_bind(max_total_tokens);
    }

    if let Some(min_total_ms) = filters.min_total_ms {
        query
            .push(" AND t_total_ms IS NOT NULL AND t_total_ms >= ")
            .push_bind(min_total_ms);
    }

    if let Some(max_total_ms) = filters.max_total_ms {
        query
            .push(" AND t_total_ms IS NOT NULL AND t_total_ms <= ")
            .push_bind(max_total_ms);
    }
}

pub(crate) fn apply_invocation_records_filters(
    query: &mut QueryBuilder<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) {
    apply_invocation_scope_filters(query, filters, source_scope, snapshot);
    apply_invocation_model_filters(query, filters);
    apply_invocation_status_filter(query, filters.status.as_deref());
    apply_invocation_dimension_filters(query, filters);
    apply_invocation_numeric_filters(query, filters);
}

pub(crate) async fn resolve_invocation_snapshot_id(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    let mut connection = pool.acquire().await?;
    resolve_invocation_snapshot_id_on_connection(&mut connection, source_scope).await
}

async fn resolve_invocation_snapshot_id_on_connection(
    connection: &mut SqliteConnection,
    source_scope: InvocationSourceScope,
) -> Result<i64> {
    #[derive(Debug, FromRow)]
    struct SnapshotRow {
        snapshot_id: Option<i64>,
    }

    let mut query =
        QueryBuilder::new("SELECT MAX(id) AS snapshot_id FROM codex_invocations WHERE 1 = 1");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    let row = query
        .build_query_as::<SnapshotRow>()
        .fetch_one(connection)
        .await?;
    Ok(row.snapshot_id.unwrap_or(0))
}

pub(crate) fn append_invocation_order_clause(
    query: &mut QueryBuilder<Sqlite>,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
) {
    let direction = sort_order.sql_keyword();
    query.push(" ORDER BY ");
    if matches!(sort_by, InvocationSortBy::Status) {
        let status_expr = invocation_display_status_sql();
        query.push("(");
        query.push(&status_expr);
        query.push(") IS NULL ASC, ");
        query.push(status_expr);
    } else {
        query.push(sort_by.sql_expr());
        query.push(" IS NULL ASC, ");
        query.push(sort_by.sql_expr());
    }
    query.push(" ");
    query.push(direction);
    match sort_by {
        InvocationSortBy::OccurredAt => {
            query.push(", id ");
            query.push(direction);
        }
        _ => {
            query.push(", occurred_at DESC, id DESC");
        }
    }
}

pub(crate) fn normalized_runtime_text(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_lowercase()
}

pub(crate) const USAGE_BREAKDOWN_UNASSIGNED_ACCOUNT_KEY: &str = "__unassigned__";

fn usage_breakdown_payload_text(payload: Option<&str>, key: &str) -> Option<String> {
    let payload = payload?;
    let value = serde_json::from_str::<Value>(payload).ok()?;
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(crate) fn normalized_usage_breakdown_model(
    model: Option<&str>,
    payload: Option<&str>,
) -> String {
    usage_breakdown_payload_text(payload, "responseModel")
        .or_else(|| {
            model
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

pub(crate) fn normalized_usage_breakdown_reasoning_effort(payload: Option<&str>) -> Option<String> {
    usage_breakdown_payload_text(payload, "reasoningEffort")
}

pub(crate) fn normalized_usage_breakdown_account_key(
    payload: Option<&str>,
    upstream_account_id: Option<i64>,
) -> (String, Option<i64>) {
    match upstream_account_id.or_else(|| crate::proxy::upstream_account_id_from_payload(payload)) {
        Some(upstream_account_id) => (
            format!("upstream:{upstream_account_id}"),
            Some(upstream_account_id),
        ),
        None => (USAGE_BREAKDOWN_UNASSIGNED_ACCOUNT_KEY.to_string(), None),
    }
}

pub(crate) fn runtime_text_equals(value: Option<&str>, expected: &str) -> bool {
    normalized_runtime_text(value) == expected.trim().to_lowercase()
}

pub(crate) fn runtime_text_list_contains(values: &[String], candidate: Option<&str>) -> bool {
    let normalized_candidate = normalized_runtime_text(candidate);
    !normalized_candidate.is_empty()
        && values
            .iter()
            .any(|value| normalized_runtime_text(Some(value.as_str())) == normalized_candidate)
}

pub(crate) fn runtime_keyword_matches(record: &ApiInvocation, keyword: &str) -> bool {
    let keyword = keyword.trim().to_lowercase();
    if keyword.is_empty() {
        return true;
    }
    [
        Some(record.invoke_id.as_str()),
        record.model.as_deref(),
        record.request_model.as_deref(),
        record.response_model.as_deref(),
        record.proxy_display_name.as_deref(),
        record.endpoint.as_deref(),
        record.failure_kind.as_deref(),
        record.error_message.as_deref(),
        record.downstream_error_message.as_deref(),
        record.prompt_cache_key.as_deref(),
        record.requester_ip.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(&keyword))
}

pub(crate) fn runtime_sticky_key(record: &ApiInvocation) -> Option<&str> {
    record
        .sticky_key
        .as_deref()
        .or(record.prompt_cache_key.as_deref())
}

pub(crate) fn runtime_upstream_scope(record: &ApiInvocation) -> &'static str {
    if runtime_text_equals(record.route_mode.as_deref(), "pool") {
        "internal"
    } else {
        "external"
    }
}

pub(crate) fn runtime_model_value(
    record: &ApiInvocation,
    target: InvocationModelTarget,
) -> Option<&str> {
    match target {
        InvocationModelTarget::Request => {
            record.request_model.as_deref().or(record.model.as_deref())
        }
        InvocationModelTarget::Response => {
            record.response_model.as_deref().or(record.model.as_deref())
        }
    }
}

pub(crate) fn runtime_model_rerouted(record: &ApiInvocation) -> bool {
    let request_model = normalized_runtime_text(record.request_model.as_deref());
    let response_model = normalized_runtime_text(record.response_model.as_deref());
    !request_model.is_empty() && !response_model.is_empty() && request_model != response_model
}

pub(crate) fn runtime_record_is_retry(record: &ApiInvocation) -> bool {
    record.pool_attempt_count.unwrap_or(1) > 1
}

pub(crate) fn runtime_record_first_token_ms(record: &ApiInvocation) -> Option<f64> {
    runtime_record_first_token_ms_with_retry(record, runtime_record_is_retry(record))
}

pub(crate) fn runtime_record_first_token_ms_with_retry(
    record: &ApiInvocation,
    is_retry: bool,
) -> Option<f64> {
    let first_token_ms = finite_nonnegative_timing(record.first_token_ms);
    if !is_retry {
        return first_token_ms;
    }

    // Runtime capture records use a synthetic row id and are rebuilt for the selected attempt.
    // Persisted rows need the attempt-table predicate instead of invocation-level timing.
    if record.id > 0 {
        return None;
    }
    if runtime_record_is_in_flight(record) {
        return (runtime_invocation_live_phase(record) == Some(INVOCATION_LIVE_PHASE_RESPONDING))
            .then_some(first_token_ms)
            .flatten();
    }

    (runtime_record_is_success_for_summary(record)
        || finite_positive_timing(record.t_upstream_stream_ms).is_some())
    .then_some(first_token_ms)
    .flatten()
}

pub(crate) fn runtime_record_live_phase(record: &ApiInvocation) -> Option<&'static str> {
    runtime_record_live_phase_with_retry(record, runtime_record_is_retry(record))
}

pub(crate) fn runtime_record_live_phase_with_retry(
    record: &ApiInvocation,
    is_retry: bool,
) -> Option<&'static str> {
    if is_retry && !runtime_record_is_retry(record) {
        let mut phase_record = record.clone();
        phase_record.pool_attempt_count = Some(2);
        effective_runtime_invocation_live_phase(&phase_record)
    } else {
        effective_runtime_invocation_live_phase(record)
    }
}

pub(crate) fn runtime_record_is_in_flight(record: &ApiInvocation) -> bool {
    matches!(
        normalized_runtime_text(record.status.as_deref()).as_str(),
        "running" | "pending"
    )
}

fn runtime_record_matches_scope_and_model(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    if source_scope == InvocationSourceScope::ProxyOnly && record.source != SOURCE_PROXY {
        return false;
    }
    if let Some(from_bound) = filters.occurred_from.as_deref()
        && record.occurred_at.as_str() < from_bound
    {
        return false;
    }
    if let Some(to_bound) = filters.occurred_to.as_deref()
        && record.occurred_at.as_str() >= to_bound
    {
        return false;
    }
    if !filters.model_values.is_empty() {
        let target = filters
            .model_target
            .unwrap_or(InvocationModelTarget::Request);
        if !runtime_text_list_contains(&filters.model_values, runtime_model_value(record, target)) {
            return false;
        }
    } else if let Some(model) = filters.model.as_deref()
        && !runtime_text_equals(record.model.as_deref(), model)
    {
        return false;
    }
    if let Some(model_rerouted) = filters.model_rerouted {
        let rerouted = runtime_model_rerouted(record);
        match model_rerouted {
            InvocationModelRerouteFilter::Rerouted if !rerouted => return false,
            InvocationModelRerouteFilter::NotRerouted if rerouted => return false,
            _ => {}
        }
    }
    true
}

fn runtime_record_failure_class(record: &ApiInvocation) -> FailureClass {
    resolve_failure_classification(
        record.status.as_deref(),
        record.error_message.as_deref(),
        record.failure_kind.as_deref(),
        record.failure_class.as_deref(),
        record.is_actionable.map(|value| if value { 1 } else { 0 }),
    )
    .failure_class
}

fn runtime_record_matches_status_filter(record: &ApiInvocation, status: Option<&str>) -> bool {
    let Some(status) = status else {
        return true;
    };
    let normalized_status = status.trim();
    if normalized_status.eq_ignore_ascii_case("failed") {
        return prompt_cache_and_timeseries_shared::prompt_invocation_status_counts_toward_terminal_totals(
            record.status.as_deref(),
        ) && runtime_record_failure_class(record) != FailureClass::None
            && !runtime_text_equals(record.status.as_deref(), "interrupted");
    }
    if normalized_status.eq_ignore_ascii_case("success")
        || normalized_status.eq_ignore_ascii_case(INVOCATION_STATUS_WARNING_SUCCESS)
    {
        return runtime_text_equals(record.status.as_deref(), normalized_status)
            && runtime_record_failure_class(record) == FailureClass::None;
    }
    runtime_text_equals(record.status.as_deref(), normalized_status)
}

fn runtime_record_matches_dimension_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
) -> bool {
    if let Some(endpoint) = filters.endpoint.as_deref()
        && !runtime_text_equals(record.endpoint.as_deref(), endpoint)
    {
        return false;
    }
    if let Some(request_id) = filters.request_id.as_deref()
        && !runtime_text_equals(Some(record.invoke_id.as_str()), request_id)
    {
        return false;
    }
    if let Some(failure_class) = filters.failure_class.as_deref()
        && !runtime_text_equals(record.failure_class.as_deref(), failure_class)
    {
        return false;
    }
    if let Some(failure_kind) = filters.failure_kind.as_deref()
        && !runtime_text_equals(record.failure_kind.as_deref(), failure_kind)
    {
        return false;
    }
    if let Some(prompt_cache_key) = filters.prompt_cache_key.as_deref()
        && !runtime_text_equals(record.prompt_cache_key.as_deref(), prompt_cache_key)
    {
        return false;
    }
    if let Some(sticky_key) = filters.sticky_key.as_deref()
        && !runtime_text_equals(runtime_sticky_key(record), sticky_key)
    {
        return false;
    }
    if let Some(upstream_scope) = filters.upstream_scope.as_deref()
        && !runtime_text_equals(Some(runtime_upstream_scope(record)), upstream_scope)
    {
        return false;
    }
    if let Some(proxy_display_name) = filters.proxy_display_name.as_deref()
        && !runtime_text_equals(record.proxy_display_name.as_deref(), proxy_display_name)
    {
        return false;
    }
    if let Some(upstream_account_id) = filters.upstream_account_id
        && record.upstream_account_id != Some(upstream_account_id)
    {
        return false;
    }
    if let Some(transport) = filters.transport.as_deref()
        && !runtime_text_equals(record.transport.as_deref(), transport)
    {
        return false;
    }
    if let Some(service_tier) = filters.service_tier.as_deref()
        && !runtime_text_equals(record.service_tier.as_deref(), service_tier)
    {
        return false;
    }
    if !filters.reasoning_effort_values.is_empty() {
        if !runtime_text_list_contains(
            &filters.reasoning_effort_values,
            record.reasoning_effort.as_deref(),
        ) {
            return false;
        }
    } else if let Some(reasoning_effort) = filters.reasoning_effort.as_deref()
        && !runtime_text_equals(record.reasoning_effort.as_deref(), reasoning_effort)
    {
        return false;
    }
    if let Some(requester_ip) = filters.requester_ip.as_deref()
        && !runtime_text_equals(record.requester_ip.as_deref(), requester_ip)
    {
        return false;
    }
    if let Some(keyword) = filters.keyword.as_deref()
        && !runtime_keyword_matches(record, keyword)
    {
        return false;
    }
    true
}

fn runtime_record_matches_numeric_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
) -> bool {
    if let Some(min_total_tokens) = filters.min_total_tokens {
        let Some(total_tokens) = record.total_tokens else {
            return false;
        };
        if total_tokens < min_total_tokens {
            return false;
        }
    }
    if let Some(max_total_tokens) = filters.max_total_tokens {
        let Some(total_tokens) = record.total_tokens else {
            return false;
        };
        if total_tokens > max_total_tokens {
            return false;
        }
    }
    if let Some(min_total_ms) = filters.min_total_ms {
        let Some(total_ms) = record.t_total_ms else {
            return false;
        };
        if total_ms < min_total_ms {
            return false;
        }
    }
    if let Some(max_total_ms) = filters.max_total_ms {
        let Some(total_ms) = record.t_total_ms else {
            return false;
        };
        if total_ms > max_total_ms {
            return false;
        }
    }
    true
}

pub(crate) fn runtime_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    runtime_record_matches_scope_and_model(record, filters, source_scope)
        && runtime_record_matches_status_filter(record, filters.status.as_deref())
        && runtime_record_matches_dimension_filters(record, filters)
        && runtime_record_matches_numeric_filters(record, filters)
}

pub(crate) fn runtime_in_flight_record_matches_filters(
    record: &ApiInvocation,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
) -> bool {
    runtime_record_is_in_flight(record)
        && runtime_record_matches_filters(record, filters, source_scope)
}

pub(crate) fn option_presence_order(
    left_some: bool,
    right_some: bool,
) -> Option<std::cmp::Ordering> {
    match (left_some, right_some) {
        (true, false) => Some(std::cmp::Ordering::Less),
        (false, true) => Some(std::cmp::Ordering::Greater),
        (false, false) => Some(std::cmp::Ordering::Equal),
        (true, true) => None,
    }
}

pub(crate) fn apply_runtime_sort_order(
    ordering: std::cmp::Ordering,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    match sort_order {
        InvocationSortOrder::Asc => ordering,
        InvocationSortOrder::Desc => ordering.reverse(),
    }
}

pub(crate) fn compare_runtime_option_i64(
    left: Option<i64>,
    right: Option<i64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default().cmp(&right.unwrap_or_default()),
            sort_order,
        )
    })
}

pub(crate) fn compare_runtime_option_f64(
    left: Option<f64>,
    right: Option<f64>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default()
                .partial_cmp(&right.unwrap_or_default())
                .unwrap_or(std::cmp::Ordering::Equal),
            sort_order,
        )
    })
}

pub(crate) fn compare_runtime_option_str(
    left: Option<&str>,
    right: Option<&str>,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    option_presence_order(left.is_some(), right.is_some()).unwrap_or_else(|| {
        apply_runtime_sort_order(
            left.unwrap_or_default().cmp(right.unwrap_or_default()),
            sort_order,
        )
    })
}

pub(crate) fn invocation_display_status_value(record: &ApiInvocation) -> Option<&str> {
    let status = record.status.as_deref().map(str::trim).unwrap_or_default();
    let failure_class = record
        .failure_class
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if status.eq_ignore_ascii_case("interrupted") {
        Some("interrupted")
    } else if matches!(
        failure_class,
        "service_failure" | "client_failure" | "client_abort"
    ) {
        Some("failed")
    } else if status.is_empty() {
        Some("unknown")
    } else {
        record.status.as_deref()
    }
}

pub(crate) fn compare_runtime_invocation_records(
    left: &ApiInvocation,
    right: &ApiInvocation,
    sort_by: InvocationSortBy,
    sort_order: InvocationSortOrder,
) -> std::cmp::Ordering {
    let primary = match sort_by {
        InvocationSortBy::OccurredAt => compare_runtime_option_str(
            Some(left.occurred_at.as_str()),
            Some(right.occurred_at.as_str()),
            sort_order,
        )
        .then_with(|| apply_runtime_sort_order(left.id.cmp(&right.id), sort_order)),
        InvocationSortBy::TotalTokens => {
            compare_runtime_option_i64(left.total_tokens, right.total_tokens, sort_order)
        }
        InvocationSortBy::Cost => compare_runtime_option_f64(left.cost, right.cost, sort_order),
        InvocationSortBy::TotalMs => {
            compare_runtime_option_f64(left.t_total_ms, right.t_total_ms, sort_order)
        }
        InvocationSortBy::TtfbMs => compare_runtime_option_f64(
            left.t_upstream_ttfb_ms,
            right.t_upstream_ttfb_ms,
            sort_order,
        ),
        InvocationSortBy::Status => compare_runtime_option_str(
            invocation_display_status_value(left),
            invocation_display_status_value(right),
            sort_order,
        ),
    };
    primary
        .then_with(|| right.occurred_at.cmp(&left.occurred_at))
        .then_with(|| right.id.cmp(&left.id))
}

pub(crate) fn should_overlay_runtime_records(request: &InvocationListRequest) -> bool {
    request.snapshot_id.is_none()
}

pub(crate) fn runtime_overlay_snapshot(state: &AppState) -> Vec<ApiInvocation> {
    state.proxy_runtime_invocations.snapshot()
}

pub(crate) async fn query_current_runtime_db_keys(
    pool: &Pool<Sqlite>,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    let mut connection = pool.acquire().await?;
    query_current_runtime_db_keys_on_connection(&mut connection, filters, source_scope, snapshot)
        .await
}

async fn query_current_runtime_db_keys_on_connection(
    connection: &mut SqliteConnection,
    filters: &InvocationRecordsFilters,
    source_scope: InvocationSourceScope,
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT invoke_id, occurred_at FROM codex_invocations WHERE 1 = 1",
    );
    apply_invocation_records_filters(&mut query, filters, source_scope, snapshot);
    query.push(" AND LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')");

    Ok(query
        .build_query_as::<RuntimeKeyRow>()
        .fetch_all(connection)
        .await?
        .into_iter()
        .map(|row| (row.invoke_id, row.occurred_at))
        .collect())
}

async fn count_stale_runtime_db_rows_before_target(
    pool: &Pool<Sqlite>,
    stale_keys: &HashSet<(String, String)>,
    target_occurred_at: &str,
    target_id: i64,
    snapshot_id: i64,
) -> Result<i64, ApiError> {
    #[derive(Debug, FromRow)]
    struct CountRow {
        total: i64,
    }

    let mut count = stale_keys
        .iter()
        .filter(|(_, occurred_at)| occurred_at.as_str() > target_occurred_at)
        .count() as i64;
    let equal_time_keys = stale_keys
        .iter()
        .filter(|(_, occurred_at)| occurred_at.as_str() == target_occurred_at)
        .collect::<Vec<_>>();
    for chunk in equal_time_keys.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT COUNT(*) AS total FROM codex_invocations WHERE id <= ",
        );
        query
            .push_bind(snapshot_id)
            .push(" AND id > ")
            .push_bind(target_id)
            .push(" AND occurred_at = ")
            .push_bind(target_occurred_at.to_string())
            .push(" AND (");
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        count += query
            .build_query_as::<CountRow>()
            .fetch_one(pool)
            .await?
            .total;
    }
    Ok(count)
}

pub(crate) async fn query_terminal_db_keys_for_runtime_records(
    pool: &Pool<Sqlite>,
    runtime_records: &[ApiInvocation],
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    let mut connection = pool.acquire().await?;
    query_terminal_db_keys_for_runtime_records_on_connection(
        &mut connection,
        runtime_records,
        snapshot,
    )
    .await
}

async fn query_terminal_db_keys_for_runtime_records_on_connection(
    connection: &mut SqliteConnection,
    runtime_records: &[ApiInvocation],
    snapshot: Option<SnapshotConstraint>,
) -> Result<HashSet<(String, String)>, ApiError> {
    #[derive(Debug, FromRow)]
    struct RuntimeKeyRow {
        invoke_id: String,
        occurred_at: String,
    }

    let keys = runtime_records
        .iter()
        .map(|record| (record.invoke_id.clone(), record.occurred_at.clone()))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if keys.is_empty() {
        return Ok(HashSet::new());
    }

    let mut terminal_keys = HashSet::new();
    for chunk in keys.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT invoke_id, occurred_at FROM codex_invocations \
             WHERE LOWER(TRIM(COALESCE(status, ''))) NOT IN ('running', 'pending') \
             AND (",
        );
        for (index, (invoke_id, occurred_at)) in chunk.iter().enumerate() {
            if index > 0 {
                query.push(" OR ");
            }
            query
                .push("(invoke_id = ")
                .push_bind(invoke_id)
                .push(" AND occurred_at = ")
                .push_bind(occurred_at)
                .push(")");
        }
        query.push(")");
        if let Some(snapshot_constraint) = snapshot {
            match snapshot_constraint {
                SnapshotConstraint::UpTo(snapshot_id) => {
                    query.push(" AND id <= ").push_bind(snapshot_id);
                }
                SnapshotConstraint::After(snapshot_id) => {
                    query.push(" AND id > ").push_bind(snapshot_id);
                }
            }
        }
        terminal_keys.extend(
            query
                .build_query_as::<RuntimeKeyRow>()
                .fetch_all(&mut *connection)
                .await?
                .into_iter()
                .map(|row| (row.invoke_id, row.occurred_at)),
        );
    }
    Ok(terminal_keys)
}
