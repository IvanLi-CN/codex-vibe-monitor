fn build_upstream_account_attempt_page_query<'query>() -> QueryBuilder<'query, Sqlite> {
    QueryBuilder::<Sqlite>::new(format!(
        r#"
        SELECT
            attempts.id,
            attempts.attempt_public_id AS attempt_id,
            attempts.invoke_id,
            attempts.occurred_at,
            attempts.endpoint,
            attempts.sticky_key,
            attempts.routing_source,
            attempts.upstream_account_id,
            accounts.display_name AS upstream_account_name,
            attempts.upstream_route_key,
            attempts.proxy_binding_key_snapshot,
            attempts.attempt_index,
            attempts.distinct_account_index,
            attempts.same_account_retry_index,
            attempts.requester_ip,
            {model_sql} AS model,
            {request_model_sql} AS request_model,
            attempts.upstream_request_model,
            attempts.model_mapping_pattern,
            {response_model_sql} AS response_model,
            {compaction_request_kind_sql} AS compaction_request_kind,
            {compaction_response_kind_sql} AS compaction_response_kind,
            {image_intent_sql} AS image_intent,
            attempts.started_at,
            attempts.finished_at,
            attempts.status,
            COALESCE(
                attempts.phase,
                CASE
                    WHEN attempts.status = 'pending' THEN 'sending_request'
                    WHEN attempts.status = 'success' THEN 'completed'
                    ELSE 'failed'
                END
            ) AS phase,
            attempts.http_status,
            attempts.downstream_http_status,
            attempts.failure_kind,
            attempts.error_message,
            attempts.downstream_error_message,
            attempts.connect_latency_ms,
            {final_attempt_first_token_ms_sql},
            attempts.first_byte_latency_ms,
            attempts.stream_latency_ms,
            attempts.upstream_request_id,
            COALESCE(inv.request_raw_codec, 'identity') AS downstream_request_content_encoding,
            attempts.upstream_request_compression_algorithm,
            attempts.upstream_request_compression_mode,
            attempts.upstream_request_logical_body_bytes,
            attempts.upstream_request_transmitted_body_bytes,
            attempts.upstream_request_header_bytes_approx,
            attempts.upstream_response_body_bytes,
            attempts.upstream_response_header_bytes_approx,
            attempts.compact_support_status,
            attempts.compact_support_reason,
            attempts.created_at
        FROM pool_upstream_request_attempts AS attempts
        LEFT JOIN pool_upstream_accounts AS accounts
            ON accounts.id = attempts.upstream_account_id
        LEFT JOIN codex_invocations AS inv
            ON inv.invoke_id = attempts.invoke_id
           AND inv.occurred_at = attempts.occurred_at
        "#,
        model_sql = ACCOUNT_ATTEMPT_MODEL_SQL,
        request_model_sql = ACCOUNT_ATTEMPT_REQUEST_MODEL_SQL,
        response_model_sql = ACCOUNT_ATTEMPT_RESPONSE_MODEL_SQL,
        compaction_request_kind_sql = ACCOUNT_ATTEMPT_COMPACTION_REQUEST_KIND_SQL,
        compaction_response_kind_sql = ACCOUNT_ATTEMPT_COMPACTION_RESPONSE_KIND_SQL,
        image_intent_sql = ACCOUNT_ATTEMPT_IMAGE_INTENT_SQL,
        final_attempt_first_token_ms_sql =
            crate::final_pool_attempt_first_token_ms_sql("attempts", "inv"),
    ))
}

async fn load_upstream_account_attempt_page(
    state: &AppState,
    account_id: i64,
    page: usize,
    page_size: usize,
    filters: &UpstreamAccountAttemptFilters,
) -> Result<UpstreamAccountAttemptListResponse> {
    let pool = &state.pool;
    let cutoff = shanghai_local_cutoff_string(ACCOUNT_ATTEMPT_RETENTION_DAYS);
    let total = load_upstream_account_attempt_total(pool, account_id, &cutoff, filters).await?;
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let mut query = build_upstream_account_attempt_page_query();
    push_upstream_account_attempt_scope(&mut query, account_id, &cutoff);
    push_upstream_account_attempt_filters(&mut query, filters, true);
    query
        .push(" ORDER BY attempts.occurred_at DESC, attempts.id DESC LIMIT ")
        .push_bind(page_size as i64)
        .push(" OFFSET ")
        .push_bind(offset as i64);
    let mut items = query
        .build_query_as::<ApiPoolUpstreamRequestAttempt>()
        .fetch_all(pool)
        .await?;
    sanitize_pool_attempt_timing_fields(&mut items);
    hydrate_pool_attempt_request_compression_fields(&mut items);
    hydrate_upstream_account_attempt_workflow_entries(state, &mut items)
        .await
        .map_err(|err| anyhow!("failed to hydrate account attempt workflow entries: {err:?}"))?;
    let sticky_key_options =
        load_upstream_account_attempt_sticky_key_options(pool, account_id, &cutoff, filters)
            .await?;
    Ok(UpstreamAccountAttemptListResponse {
        items,
        sticky_key_options,
        total,
        page,
        page_size,
    })
}
