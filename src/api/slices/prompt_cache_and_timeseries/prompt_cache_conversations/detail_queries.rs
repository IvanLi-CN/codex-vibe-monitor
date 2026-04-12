use super::*;

pub(crate) async fn query_prompt_cache_conversation_events(
    pool: &Pool<Sqlite>,
    range_start_bound: &str,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationEventRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT occurred_at, COALESCE(status, '') AS status, \
         error_message, ",
    );
    query
        .push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, COALESCE(total_tokens, 0) AS request_tokens, ")
        .push(KEY_EXPR)
        .push(
            " AS prompt_cache_key \
             FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(range_start_bound)
        .push(" AND ");
    if let Some(snapshot) = snapshot {
        let snapshot_filter = PromptCacheConversationSnapshotFilter {
            snapshot_upper_bound: snapshot.snapshot_upper_bound.to_string(),
            snapshot_boundary_row_id_ceiling: snapshot.snapshot_boundary_row_id_ceiling,
        };
        push_snapshot_invocation_visibility_clause(
            &mut query,
            "occurred_at",
            "id",
            Some(&snapshot_filter),
        );
        query.push(" AND ");
    }
    query.push(KEY_EXPR).push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(" ORDER BY prompt_cache_key ASC, occurred_at ASC, id ASC");

    query
        .build_query_as::<PromptCacheConversationEventRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_recent_invocations(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    limit_per_key: i64,
    snapshot: Option<&PromptCacheConversationHydrationSnapshot<'_>>,
) -> Result<Vec<PromptCacheConversationInvocationPreviewRow>> {
    if selected_keys.is_empty() || limit_per_key <= 0 {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    let mut query =
        QueryBuilder::<Sqlite>::new("WITH ranked AS (SELECT id, invoke_id, occurred_at, ");
    query
        .push(invocation_display_status_sql())
        .push(" AS status, ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" AS failure_class, ")
        .push(INVOCATION_ROUTE_MODE_SQL)
        .push(" AS route_mode, model, COALESCE(total_tokens, 0) AS total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, ")
        .push(INVOCATION_REASONING_EFFORT_SQL)
        .push(" AS reasoning_effort, error_message, ")
        .push(INVOCATION_FAILURE_KIND_SQL)
        .push(" AS failure_kind, CASE WHEN ")
        .push(INVOCATION_RESOLVED_FAILURE_CLASS_SQL)
        .push(" = 'service_failure' THEN 1 ELSE 0 END AS is_actionable, ")
        .push(INVOCATION_PROXY_DISPLAY_SQL)
        .push(" AS proxy_display_name, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_ID_SQL)
        .push(" AS upstream_account_id, ")
        .push(INVOCATION_UPSTREAM_ACCOUNT_NAME_SQL)
        .push(" AS upstream_account_name, ")
        .push(INVOCATION_RESPONSE_CONTENT_ENCODING_SQL)
        .push(
            " AS response_content_encoding, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text' \
                 THEN json_extract(payload, '$.requestedServiceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text' \
                 THEN json_extract(payload, '$.requested_service_tier') END AS requested_service_tier, \
             CASE \
               WHEN json_valid(payload) AND json_type(payload, '$.serviceTier') = 'text' \
                 THEN json_extract(payload, '$.serviceTier') \
               WHEN json_valid(payload) AND json_type(payload, '$.service_tier') = 'text' \
                 THEN json_extract(payload, '$.service_tier') END AS service_tier, \
             ",
        )
        .push(INVOCATION_BILLING_SERVICE_TIER_SQL)
        .push(
            " AS billing_service_tier, \
             t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, \
             t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, ",
        )
        .push(INVOCATION_DOWNSTREAM_STATUS_CODE_SQL)
        .push(" AS downstream_status_code, ")
        .push(INVOCATION_DOWNSTREAM_ERROR_MESSAGE_SQL)
        .push(" AS downstream_error_message, ")
        .push(INVOCATION_ENDPOINT_SQL)
        .push(" AS endpoint, ")
        .push(KEY_EXPR)
        .push(" AS prompt_cache_key, ROW_NUMBER() OVER (PARTITION BY ")
        .push(KEY_EXPR)
        .push(" ORDER BY occurred_at DESC, id DESC) AS row_number FROM codex_invocations WHERE ")
        .push(KEY_EXPR)
        .push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if let Some(snapshot) = snapshot {
        let snapshot_filter = PromptCacheConversationSnapshotFilter {
            snapshot_upper_bound: snapshot.snapshot_upper_bound.to_string(),
            snapshot_boundary_row_id_ceiling: snapshot.snapshot_boundary_row_id_ceiling,
        };
        query.push(" AND ");
        push_snapshot_invocation_visibility_clause(
            &mut query,
            "occurred_at",
            "id",
            Some(&snapshot_filter),
        );
    }

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(") SELECT prompt_cache_key, id, invoke_id, occurred_at, status, failure_class, route_mode, model, total_tokens, cost, source, input_tokens, output_tokens, cache_input_tokens, reasoning_tokens, reasoning_effort, error_message, downstream_status_code, downstream_error_message, failure_kind, is_actionable, proxy_display_name, upstream_account_id, upstream_account_name, response_content_encoding, requested_service_tier, service_tier, billing_service_tier, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms, t_upstream_ttfb_ms, t_upstream_stream_ms, t_resp_parse_ms, t_persist_ms, t_total_ms, endpoint FROM ranked WHERE row_number <= ")
        .push_bind(limit_per_key)
        .push(" ORDER BY prompt_cache_key ASC, occurred_at DESC, id DESC");

    query
        .build_query_as::<PromptCacheConversationInvocationPreviewRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_upstream_account_summaries_at_snapshot(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
    snapshot_hour_start_epoch: i64,
    snapshot_hour_start_bound: &str,
    snapshot: &PromptCacheConversationHydrationSnapshot<'_>,
) -> Result<Vec<PromptCacheConversationUpstreamAccountSummaryRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    const KEY_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.promptCacheKey') AS TEXT)) END";
    const UPSTREAM_ACCOUNT_ID_EXPR: &str = "CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END";
    const UPSTREAM_ACCOUNT_NAME_EXPR: &str = "CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END";
    const UPSTREAM_ACCOUNT_KEY_EXPR: &str = "CASE \
            WHEN CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END IS NOT NULL \
             AND CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END IS NOT NULL \
             AND CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END <> '' \
              THEN 'id:' || CAST(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS TEXT) || '|name:' || CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END \
            WHEN CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END IS NOT NULL \
              THEN 'id:' || CAST(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.upstreamAccountId') AS INTEGER) END AS TEXT) \
            WHEN CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END IS NOT NULL \
             AND CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END <> '' \
              THEN 'name:' || CASE WHEN json_valid(payload) THEN TRIM(CAST(json_extract(payload, '$.upstreamAccountName') AS TEXT)) END \
            ELSE 'unknown' \
         END";

    let mut query = QueryBuilder::<Sqlite>::new(
        "WITH historical AS (\
            SELECT prompt_cache_key, \
                   upstream_account_key, \
                   upstream_account_id, \
                   upstream_account_name, \
                   SUM(request_count) AS request_count, \
                   SUM(total_tokens) AS total_tokens, \
                   SUM(total_cost) AS total_cost, \
                   MAX(last_seen_at) AS last_activity_at \
              FROM prompt_cache_upstream_account_hourly \
             WHERE prompt_cache_key IN (",
    );

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }

    query
        .push(") AND bucket_start_epoch < ")
        .push_bind(snapshot_hour_start_epoch);

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query.push(
        " GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name\
         ), current_hour_live AS (\
            SELECT ",
    );
    query
        .push(KEY_EXPR)
        .push(" AS prompt_cache_key, ")
        .push(UPSTREAM_ACCOUNT_KEY_EXPR)
        .push(" AS upstream_account_key, ")
        .push(UPSTREAM_ACCOUNT_ID_EXPR)
        .push(" AS upstream_account_id, ")
        .push(UPSTREAM_ACCOUNT_NAME_EXPR)
        .push(
            " AS upstream_account_name, \
                   COUNT(*) AS request_count, \
                   COALESCE(SUM(COALESCE(total_tokens, 0)), 0) AS total_tokens, \
                   COALESCE(SUM(COALESCE(cost, 0.0)), 0.0) AS total_cost, \
                   MAX(occurred_at) AS last_activity_at \
              FROM codex_invocations \
             WHERE occurred_at >= ",
        )
        .push_bind(snapshot_hour_start_bound)
        .push(" AND ");
    let snapshot_filter = PromptCacheConversationSnapshotFilter {
        snapshot_upper_bound: snapshot.snapshot_upper_bound.to_string(),
        snapshot_boundary_row_id_ceiling: snapshot.snapshot_boundary_row_id_ceiling,
    };
    push_snapshot_invocation_visibility_clause(
        &mut query,
        "occurred_at",
        "id",
        Some(&snapshot_filter),
    );
    query.push(" AND ").push(KEY_EXPR).push(" IN (");

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");

    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(" GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name\
         ), combined AS (\
            SELECT prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name, request_count, total_tokens, total_cost, last_activity_at \
              FROM historical \
            UNION ALL \
            SELECT prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name, request_count, total_tokens, total_cost, last_activity_at \
              FROM current_hour_live\
         ) \
         SELECT prompt_cache_key, \
                upstream_account_id, \
                upstream_account_name, \
                SUM(request_count) AS request_count, \
                SUM(total_tokens) AS total_tokens, \
                SUM(total_cost) AS total_cost, \
                MAX(last_activity_at) AS last_activity_at \
           FROM combined \
          GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name \
          ORDER BY prompt_cache_key ASC, last_activity_at DESC, upstream_account_name DESC, upstream_account_id DESC");

    query
        .build_query_as::<PromptCacheConversationUpstreamAccountSummaryRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub(crate) async fn query_prompt_cache_conversation_upstream_account_summaries(
    pool: &Pool<Sqlite>,
    source_scope: InvocationSourceScope,
    selected_keys: &[String],
) -> Result<Vec<PromptCacheConversationUpstreamAccountSummaryRow>> {
    if selected_keys.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT prompt_cache_key, \
             upstream_account_id, \
             upstream_account_name, \
             SUM(request_count) AS request_count, \
             SUM(total_tokens) AS total_tokens, \
             SUM(total_cost) AS total_cost, \
             MAX(last_seen_at) AS last_activity_at \
         FROM prompt_cache_upstream_account_hourly \
         WHERE prompt_cache_key IN (",
    );

    {
        let mut separated = query.separated(", ");
        for key in selected_keys {
            separated.push_bind(key);
        }
    }
    query.push(")");
    if source_scope == InvocationSourceScope::ProxyOnly {
        query.push(" AND source = ").push_bind(SOURCE_PROXY);
    }

    query
        .push(
            " GROUP BY prompt_cache_key, upstream_account_key, upstream_account_id, upstream_account_name \
              ORDER BY prompt_cache_key ASC, last_activity_at DESC, upstream_account_name DESC, upstream_account_id DESC",
        )
        .build_query_as::<PromptCacheConversationUpstreamAccountSummaryRow>()
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}
