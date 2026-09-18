pub(crate) async fn list_upstream_account_action_events_from_params(
    state: Arc<AppState>,
    params: ListUpstreamAccountActionEventsQuery,
) -> Result<Json<UpstreamAccountActionEventListResponse>, (StatusCode, String)> {
    let kind_filter = normalize_upstream_account_kind_filter(params.kind.as_deref())
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let page = normalize_upstream_account_list_page(params.page);
    let page_size = normalize_upstream_account_list_page_size(params.page_size);
    let result_filter =
        normalize_upstream_account_action_event_result_filter(params.result.as_deref())
            .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let account_filter = normalize_optional_search_filter(params.account.as_deref());
    let group_filter = normalize_optional_search_filter(params.group.as_deref());
    let proxy_key_filter = normalize_optional_exact_filter(params.proxy_key.as_deref());

    let mut conditions = Vec::new();
    let mut binds: Vec<String> = Vec::new();

    if let Some(kind) = kind_filter {
        conditions.push("account.kind = ?".to_string());
        binds.push(kind.to_string());
    }

    if let Some(filter) = account_filter.as_deref() {
        conditions.push(
            "(lower(COALESCE(event.account_display_name, account.display_name)) LIKE lower(?) OR lower(COALESCE(account.email, '')) LIKE lower(?) OR CAST(event.account_id AS TEXT) LIKE ?)"
                .to_string(),
        );
        let wildcard = format!("%{filter}%");
        binds.push(wildcard.clone());
        binds.push(wildcard.clone());
        binds.push(wildcard);
    }
    if let Some(filter) = group_filter.as_deref() {
        conditions.push(
            "lower(COALESCE(event.account_group_name, account.group_name, '')) LIKE lower(?)"
                .to_string(),
        );
        binds.push(format!("%{filter}%"));
    }
    if let Some(filter) = proxy_key_filter.as_deref() {
        conditions.push("lower(COALESCE(event.forward_proxy_key, '')) = lower(?)".to_string());
        binds.push(filter.to_string());
    }
    if let Some(filter) = result_filter.as_deref() {
        conditions.push("lower(COALESCE(event.result, '')) = lower(?)".to_string());
        binds.push(filter.to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let total_sql = format!(
        r#"
        SELECT COUNT(*)
        FROM pool_upstream_account_events event
        INNER JOIN pool_upstream_accounts account ON account.id = event.account_id
        {where_clause}
        "#
    );
    let mut total_query = sqlx::query_scalar::<_, i64>(&total_sql);
    for bind in &binds {
        total_query = total_query.bind(bind);
    }
    let total = total_query
        .fetch_one(&state.pool)
        .await
        .map_err(internal_error_tuple)? as usize;

    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let items_sql = build_action_event_items_sql(&where_clause);
    let mut items_query = sqlx::query_as::<_, UpstreamAccountActionEventRow>(&items_sql);
    for bind in &binds {
        items_query = items_query.bind(bind);
    }
    let rows = items_query
        .bind(page_size as i64)
        .bind(offset as i64)
        .fetch_all(&state.pool)
        .await
        .map_err(internal_error_tuple)?;
    let items = rows
        .iter()
        .map(build_action_event_from_row)
        .collect::<Vec<_>>();

    Ok(Json(UpstreamAccountActionEventListResponse {
        items,
        total,
        page,
        page_size,
    }))
}

fn build_action_event_items_sql(where_clause: &str) -> String {
    format!(
        r#"
        SELECT
            event.id,
            event.occurred_at,
            event.action,
            event.source,
            COALESCE(event.account_display_name, account.display_name) AS account_display_name,
            COALESCE(event.account_group_name, account.group_name) AS account_group_name,
            event.forward_proxy_key,
            event.forward_proxy_display_name,
            event.forward_proxy_egress_ip,
            event.result,
            event.result_description,
            event.reason_code,
            event.reason_message,
            event.http_status,
            event.failure_kind,
            event.invoke_id,
            attempts.attempt_public_id,
            event.sticky_key,
            COALESCE(
                event.model,
                attempts.request_model,
                (
                    SELECT COALESCE(
                        NULLIF(TRIM(CASE
                            WHEN json_valid(fallback_invocation.payload)
                                THEN CAST(json_extract(fallback_invocation.payload, '$.requestModel') AS TEXT)
                        END), ''),
                        NULLIF(TRIM(fallback_invocation.model), '')
                    )
                    FROM codex_invocations fallback_invocation
                    WHERE fallback_invocation.invoke_id = event.invoke_id
                      AND ABS(
                            julianday(
                                fallback_invocation.occurred_at,
                                CASE WHEN instr(fallback_invocation.occurred_at, 'T') > 0
                                    THEN '+0 hours' ELSE '-8 hours' END
                            ) -
                            julianday(
                                COALESCE(attempts.occurred_at, event.occurred_at),
                                CASE WHEN instr(COALESCE(attempts.occurred_at, event.occurred_at), 'T') > 0
                                    THEN '+0 hours' ELSE '-8 hours' END
                            )
                          ) = (
                            SELECT MIN(ABS(
                                julianday(
                                    candidate.occurred_at,
                                    CASE WHEN instr(candidate.occurred_at, 'T') > 0
                                        THEN '+0 hours' ELSE '-8 hours' END
                                ) -
                                julianday(
                                    COALESCE(attempts.occurred_at, event.occurred_at),
                                    CASE WHEN instr(COALESCE(attempts.occurred_at, event.occurred_at), 'T') > 0
                                        THEN '+0 hours' ELSE '-8 hours' END
                                )
                            ))
                            FROM codex_invocations candidate
                            WHERE candidate.invoke_id = event.invoke_id
                          )
                    ORDER BY fallback_invocation.id DESC
                    LIMIT 1
                )
            ) AS model,
            event.model_route_state_before,
            event.model_route_state_after,
            event.model_route_priority_before,
            event.model_route_priority_after,
            event.model_route_failure_count,
            event.model_route_cooldown_until,
            NULL AS blocked_binding_json,
            event.created_at
        FROM pool_upstream_account_events event
        INNER JOIN pool_upstream_accounts account ON account.id = event.account_id
        LEFT JOIN pool_upstream_request_attempts attempts ON attempts.id = event.attempt_id
        {where_clause}
        ORDER BY event.occurred_at DESC, event.id DESC
        LIMIT ? OFFSET ?
        "#
    )
}
