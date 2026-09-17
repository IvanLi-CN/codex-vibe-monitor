#[tokio::test]
pub(crate) async fn update_upstream_account_group_disabling_retry_clears_retry_count_and_deletes_empty_row()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "LATAM Key",
        "sk-latam",
        Some("latam"),
        None,
        None,
    )
    .await;

    let initial_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "upstream429RetryEnabled": true,
        "upstream429MaxRetries": 4
    }))
    .expect("deserialize initial group payload");
    let _ = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(initial_payload),
    )
    .await
    .expect("save initial group retry settings");

    let disable_payload: UpdateUpstreamAccountGroupRequest = serde_json::from_value(json!({
        "upstream429RetryEnabled": false
    }))
    .expect("deserialize disable payload");
    let Json(updated) = update_upstream_account_group(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path("latam".to_string()),
        Json(disable_payload),
    )
    .await
    .expect("disable group retry settings");
    let updated_json = serde_json::to_value(updated).expect("serialize updated group");
    assert_eq!(
        updated_json["upstream429RetryEnabled"].as_bool(),
        Some(false)
    );
    assert_eq!(updated_json["upstream429MaxRetries"].as_u64(), Some(0));

    let persisted = sqlx::query_as::<_, (i64, i64, String)>(
        r#"
        SELECT
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            bound_proxy_keys_json
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        "#,
    )
    .bind("latam")
    .fetch_one(&state.pool)
    .await
    .expect("load persisted group metadata row");
    assert_eq!(persisted.0, 0);
    assert_eq!(persisted.1, 0);
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&persisted.2).expect("decode bound proxy keys"),
        test_required_group_bound_proxy_keys()
    );
}

#[tokio::test]
pub(crate) async fn update_upstream_account_clears_mother_without_promoting_group_peers() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let primary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Primary",
        "sk-primary",
        Some("prod"),
        Some(true),
        None,
    )
    .await;
    let secondary_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Secondary",
        "sk-secondary",
        Some("prod"),
        Some(false),
        None,
    )
    .await;

    let payload: UpdateUpstreamAccountRequest = serde_json::from_value(json!({
        "isMother": false,
    }))
    .expect("deserialize update request");
    let _ = update_upstream_account(
        State(state.clone()),
        HeaderMap::new(),
        axum::extract::Path(primary_id),
        Json(payload),
    )
    .await
    .expect("clear mother flag");

    let flags: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT id, is_mother FROM pool_upstream_accounts WHERE id IN (?1, ?2) ORDER BY id ASC",
    )
    .bind(primary_id)
    .bind(secondary_id)
    .fetch_all(&state.pool)
    .await
    .expect("load mother flags");

    assert_eq!(flags, vec![(primary_id, 0), (secondary_id, 0)]);
}

#[tokio::test]
pub(crate) async fn create_oauth_login_session_persists_mother_flag() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let payload: CreateOauthLoginSessionRequest = serde_json::from_value(json!({
        "displayName": "OAuth Mother",
        "groupName": "prod",
        "groupBoundProxyKeys": test_required_group_bound_proxy_keys(),
        "isMother": true,
    }))
    .expect("deserialize oauth session request");

    let _ = create_oauth_login_session(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("create oauth login session");

    let stored_flag: i64 = sqlx::query_scalar(
        r#"
        SELECT is_mother
        FROM pool_oauth_login_sessions
        WHERE display_name = ?1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind("OAuth Mother")
    .fetch_one(&state.pool)
    .await
    .expect("load oauth session mother flag");

    assert_eq!(stored_flag, 1);
}

#[tokio::test]
pub(crate) async fn create_oauth_login_session_relink_preserves_existing_metadata() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    ensure_upstream_accounts_schema(&state.pool)
        .await
        .expect("ensure upstream account schema");

    let now_iso = format_utc_iso(Utc::now());
    let account_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_upstream_accounts (
            kind, provider, display_name, group_name, is_mother, note, status, enabled,
            email, chatgpt_account_id, chatgpt_user_id, plan_type, masked_api_key,
            encrypted_credentials, token_expires_at, last_refreshed_at, last_synced_at,
            last_successful_sync_at, last_error, last_error_at, local_primary_limit,
            local_secondary_limit, local_limit_unit, created_at, updated_at
        ) VALUES (
            ?1, ?2, ?3, ?4, 1, ?5, ?6, 1,
            NULL, NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, NULL,
            NULL, NULL, NULL, NULL,
            NULL, NULL, ?7, ?7
        ) RETURNING id
        "#,
    )
    .bind("oauth_codex")
    .bind("codex")
    .bind("Existing OAuth")
    .bind("prod")
    .bind("Keep this note")
    .bind("active")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert oauth account");
    ensure_test_group_binding(&state.pool, "prod", None).await;

    let payload: CreateOauthLoginSessionRequest = serde_json::from_value(json!({
        "accountId": account_id,
    }))
    .expect("deserialize relink payload");
    let _ = create_oauth_login_session(State(state.clone()), HeaderMap::new(), Json(payload))
        .await
        .expect("create relink session");

    let stored: (Option<String>, Option<String>, i64, Option<String>) = sqlx::query_as(
        r#"
        SELECT display_name, group_name, is_mother, note
        FROM pool_oauth_login_sessions
        WHERE account_id = ?1
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(account_id)
    .fetch_one(&state.pool)
    .await
    .expect("load relink session");

    assert_eq!(stored.0.as_deref(), Some("Existing OAuth"));
    assert_eq!(stored.1.as_deref(), Some("prod"));
    assert_eq!(stored.2, 1);
    assert_eq!(stored.3.as_deref(), Some("Keep this note"));
}

pub(crate) fn test_stage_timings() -> StageTimings {
    StageTimings {
        t_total_ms: 0.0,
        t_req_read_ms: 0.0,
        t_req_parse_ms: 0.0,
        t_upstream_connect_ms: 0.0,
        t_upstream_ttfb_ms: 0.0,
        first_token_ms: None,
        t_upstream_stream_ms: 0.0,
        t_resp_parse_ms: 0.0,
        t_persist_ms: 0.0,
    }
}

pub(crate) fn test_proxy_capture_record(invoke_id: &str, occurred_at: &str) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: Some("gpt-5.2-codex".to_string()),
        usage: ParsedUsage {
            input_tokens: Some(12),
            output_tokens: Some(3),
            cache_input_tokens: Some(2),
            reasoning_tokens: Some(0),
            total_tokens: Some(15),
        },
        cost: Some(0.0123),
        cost_breakdown: None,
        cost_estimated: true,
        price_version: Some("unit-test".to_string()),
        status: "success".to_string(),
        error_message: None,
        failure_kind: None,
        payload: Some(
            "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":false,\"requesterIp\":\"198.51.100.77\",\"promptCacheKey\":\"pck-broadcast-1\",\"routeMode\":\"pool\",\"upstreamAccountId\":17,\"upstreamAccountName\":\"pool-account-17\",\"responseContentEncoding\":\"gzip, br\",\"requestedServiceTier\":\"priority\",\"reasoningEffort\":\"high\",\"proxyDisplayName\":\"jp-relay-01\"}"
                .to_string(),
        ),
        raw_response: "{}".to_string(),
        response_body_preview_enabled: true,
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: test_stage_timings(),
    }
}

#[test]
pub(crate) fn runtime_pool_snapshot_serializes_request_compression_from_the_active_attempt() {
    let mut record = test_proxy_capture_record("runtime-compression", "2026-07-30 10:00:00");
    set_proxy_capture_record_request_compression_algorithm(&mut record, Some("zstd"));

    let invocation = api_invocation_from_runtime_record(&record);
    assert_eq!(
        invocation.request_compression_algorithm.as_deref(),
        Some("zstd")
    );
}

pub(crate) async fn seed_success_invocation_for_records_page(
    state: &AppState,
    invoke_id: &str,
    occurred_at: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            input_tokens,
            output_tokens,
            cache_input_tokens,
            total_tokens,
            cost,
            status,
            raw_response,
            detail_level
        )
        VALUES (?1, ?2, 'proxy', 1, 2, 0, 3, 0.01, 'success', '', 'full')
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .execute(&state.pool)
    .await
    .expect("seed records page invocation");
}

struct RuntimeSnapshotSpec<'a> {
    account_id: i64,
    account_name: &'a str,
    proxy_name: &'a str,
    attempt_count: i64,
    distinct_count: i64,
    timings: [f64; 4],
}

fn runtime_overlay_request_info() -> RequestCaptureInfo {
    RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-running".to_string()),
        requested_service_tier: Some("priority".to_string()),
        reasoning_effort: Some("high".to_string()),
        compaction_request_kind: Some(CompactionKind::RemoteV2),
        is_stream: true,
        ..RequestCaptureInfo::default()
    }
}

fn build_runtime_overlay_record(
    invoke_id: &str,
    occurred_at: &str,
    request_info: &RequestCaptureInfo,
    spec: RuntimeSnapshotSpec<'_>,
) -> ProxyCaptureRecord {
    build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-running"),
        true,
        Some(spec.account_id),
        Some(spec.account_name),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some(spec.proxy_name),
        Some(spec.attempt_count as usize),
        Some(spec.distinct_count as usize),
        None,
        Some("gzip"),
        spec.timings[0],
        spec.timings[1],
        spec.timings[2],
        spec.timings[3],
    )
}

async fn assert_initial_runtime_broadcast(
    rx: &mut tokio::sync::broadcast::Receiver<BroadcastPayload>,
) {
    let record = match rx
        .recv()
        .await
        .expect("runtime snapshot payload should arrive")
    {
        BroadcastPayload::Records { records } => {
            assert_eq!(records.len(), 1);
            records.into_iter().next().expect("single running record")
        }
        other => panic!("expected records payload, got {other:?}"),
    };
    assert_eq!(record.id, 0);
    assert_eq!(record.status.as_deref(), Some("running"));
    assert_eq!(record.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(record.compaction_request_kind.as_deref(), Some("remote_v2"));
    assert_eq!(record.compaction_response_kind, None);
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
    assert_eq!(record.upstream_account_id, Some(17));
    assert_eq!(
        record.upstream_account_name.as_deref(),
        Some("pool-account-17")
    );
    assert_eq!(record.response_content_encoding.as_deref(), Some("gzip"));
    assert_eq!(record.pool_attempt_count, Some(3));
    assert_eq!(record.pool_distinct_account_count, Some(2));
    assert_eq!(record.pool_attempt_terminal_reason, None);
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-running"));
    assert_eq!(record.billing_service_tier, None);
    assert_eq!(record.t_total_ms, None);
    assert_eq!(record.t_req_read_ms, Some(22.0));
    assert_eq!(record.t_req_parse_ms, Some(4.0));
    assert_eq!(record.t_upstream_connect_ms, Some(330.0));
    assert_eq!(record.t_upstream_ttfb_ms, Some(120.0));
}

async fn assert_no_runtime_db_row(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
    context: &str,
) {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM codex_invocations WHERE invoke_id = ?1 AND occurred_at = ?2",
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .unwrap_or_else(|error| panic!("count runtime db rows {context}: {error}"));
    assert_eq!(
        count, 0,
        "runtime snapshot should remain memory-only {context}"
    );
}

async fn assert_runtime_overlay_filters(state: &Arc<AppState>, invoke_id: &str) {
    let query = |scope: Option<&str>, sticky: Option<&str>, max_tokens, max_ms| ListQuery {
        request_id: Some(invoke_id.to_string()),
        upstream_scope: scope.map(str::to_string),
        sticky_key: sticky.map(str::to_string),
        max_total_tokens: max_tokens,
        max_total_ms: max_ms,
        page_size: Some(1),
        ..Default::default()
    };
    let Json(base) = list_invocations(State(state.clone()), Query(query(None, None, None, None)))
        .await
        .expect("running invocation should be visible through memory overlay");
    assert_eq!((base.total, base.records.len()), (1, 1));
    assert_eq!(base.records[0].id, 0);
    assert_eq!(base.records[0].status.as_deref(), Some("running"));
    assert_eq!(
        base.records[0].compaction_request_kind.as_deref(),
        Some("remote_v2")
    );
    assert_eq!(base.records[0].pool_attempt_count, Some(3));
    assert_eq!(base.records[0].pool_distinct_account_count, Some(2));
    let Json(internal) = list_invocations(
        State(state.clone()),
        Query(query(Some("internal"), None, None, None)),
    )
    .await
    .expect("internal scope should include pool runtime row");
    assert_eq!(internal.total, 1);
    let Json(external) = list_invocations(
        State(state.clone()),
        Query(query(Some("external"), None, None, None)),
    )
    .await
    .expect("external scope should exclude pool runtime row");
    assert_eq!(external.total, 0);
    let Json(sticky) = list_invocations(
        State(state.clone()),
        Query(query(None, Some("pck-running"), None, None)),
    )
    .await
    .expect("sticky filter should use fallback semantics");
    assert_eq!(sticky.total, 1);
    let Json(max_tokens) = list_invocations(
        State(state.clone()),
        Query(query(None, None, Some(0), None)),
    )
    .await
    .expect("max tokens should match db null semantics");
    assert_eq!(max_tokens.total, 0);
    let Json(max_ms) = list_invocations(
        State(state.clone()),
        Query(query(None, None, None, Some(0.0))),
    )
    .await
    .expect("max total ms should match db null semantics");
    assert_eq!(max_ms.total, 0);
}

async fn assert_runtime_overlay_pagination(state: &Arc<AppState>) {
    for index in 0..60 {
        seed_success_invocation_for_records_page(
            state.as_ref(),
            &format!("page-db-{index:02}"),
            &format!("2026-03-17 18:12:{:02}", index % 60),
        )
        .await;
    }
    let Json(page) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            page: Some(2),
            page_size: Some(50),
            sort_by: Some("occurredAt".to_string()),
            sort_order: Some("desc".to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("runtime overlay should preserve records pagination");
    assert_eq!((page.total, page.records.len()), (61, 11));
    assert!(
        page.records
            .iter()
            .any(|record| record.invoke_id == "page-db-10")
    );
}

async fn assert_runtime_account_switch(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
    request_info: &RequestCaptureInfo,
) {
    let switched = build_runtime_overlay_record(
        invoke_id,
        occurred_at,
        request_info,
        RuntimeSnapshotSpec {
            account_id: 23,
            account_name: "pool-account-23",
            proxy_name: "jp-relay-02",
            attempt_count: 4,
            distinct_count: 3,
            timings: [26.0, 6.0, 350.0, 140.0],
        },
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(state, switched)
        .await
        .expect("account-switched runtime snapshot should refresh memory");
    let account_records = |account_id| ListQuery {
        request_id: Some(invoke_id.to_string()),
        upstream_account_id: Some(account_id),
        page_size: Some(1),
        ..Default::default()
    };
    let Json(old_records) = list_invocations(State(state.clone()), Query(account_records(17)))
        .await
        .expect("old account rows should be hidden after switch");
    assert_eq!(old_records.total, 0);
    let Json(new_records) = list_invocations(State(state.clone()), Query(account_records(23)))
        .await
        .expect("new account should expose runtime row");
    assert_eq!((new_records.total, new_records.records.len()), (1, 1));
    assert_eq!(new_records.records[0].id, 0);
    assert_eq!(new_records.records[0].upstream_account_id, Some(23));
    let Json(summary) = fetch_invocation_summary(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("summary should overlay running memory count");
    assert_eq!(
        (
            summary.total_count,
            summary.success_count,
            summary.failure_count
        ),
        (1, 0, 0)
    );
    assert_runtime_account_stats(state, 17, 0).await;
    assert_runtime_account_stats(state, 23, 1).await;
}

async fn assert_runtime_account_stats(state: &Arc<AppState>, account_id: i64, expected: i64) {
    let Json(summary) = fetch_summary_from_memory_snapshot(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("today".to_string()),
            limit: None,
            time_zone: Some("Asia/Shanghai".to_string()),
            upstream_account_id: Some(account_id),
        }),
    )
    .await
    .expect("account stats summary should load");
    assert_eq!(summary.in_progress_conversation_count, Some(expected));
    assert_eq!(summary.in_progress_retry_conversation_count, Some(expected));
}

async fn enqueue_terminal_runtime_overlay(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
) {
    state
        .sqlite_batch_writer
        .set_auto_flush_terminal_for_test(false);
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("terminal record should enqueue without waiting for sqlite");
    assert_no_runtime_db_row(state, invoke_id, occurred_at, "before terminal flush").await;
    let Json(summary) = fetch_invocation_summary(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            ..Default::default()
        }),
    )
    .await
    .expect("summary should overlay queued terminal record");
    assert_eq!(
        (
            summary.total_count,
            summary.success_count,
            summary.failure_count
        ),
        (1, 1, 0)
    );
    assert_eq!(summary.total_tokens, 15);
    assert_eq!(summary.token.total_tokens, 15);
    assert_eq!(summary.token.cache_input_tokens, 2);
    assert_eq!(summary.token.avg_tokens_per_request, 15.0);
}

async fn assert_terminal_runtime_flush(state: &Arc<AppState>, invoke_id: &str, occurred_at: &str) {
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .any(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
    );
    let Json(after_terminal) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("terminal invocation should load after flush");
    assert_eq!((after_terminal.total, after_terminal.records.len()), (1, 1));
    assert!(after_terminal.records[0].id > 0);
    assert_eq!(after_terminal.records[0].status.as_deref(), Some("success"));
    assert!(
        after_terminal.records[0]
            .t_persist_ms
            .is_some_and(|value| value >= 0.0)
    );
    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    assert!(
        state
            .proxy_runtime_invocations
            .snapshot()
            .iter()
            .all(|record| record.invoke_id != invoke_id || record.occurred_at != occurred_at)
    );
}

async fn assert_admitted_broadcast(rx: &mut tokio::sync::broadcast::Receiver<BroadcastPayload>) {
    let record = recv_next_records(rx)
        .await
        .into_iter()
        .next()
        .expect("single admitted record");
    assert_eq!(record.id, 0);
    assert_eq!(record.status.as_deref(), Some("running"));
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(record.requester_ip.as_deref(), Some("203.0.113.42"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-from-header"));
    assert_eq!(record.sticky_key.as_deref(), Some("sticky-from-header"));
    assert_eq!(record.model, None);
    assert_eq!(record.upstream_account_id, None);
}

async fn assert_enriched_broadcast(rx: &mut tokio::sync::broadcast::Receiver<BroadcastPayload>) {
    let record = recv_next_records(rx)
        .await
        .into_iter()
        .next()
        .expect("single enriched record");
    assert_eq!(record.id, 0);
    assert_eq!(record.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-from-body"));
    assert_eq!(record.sticky_key.as_deref(), Some("sticky-from-body"));
}

async fn assert_single_enriched_runtime_row(
    state: &Arc<AppState>,
    invoke_id: &str,
    occurred_at: &str,
) {
    let runtime_rows = state.proxy_runtime_invocations.snapshot();
    let matching_rows: Vec<_> = runtime_rows
        .iter()
        .filter(|record| record.invoke_id == invoke_id && record.occurred_at == occurred_at)
        .collect();
    assert_eq!(matching_rows.len(), 1);
    assert_eq!(matching_rows[0].model.as_deref(), Some("gpt-5.4"));
    assert_no_runtime_db_row(state, invoke_id, occurred_at, "after admitted snapshots").await;
}

#[tokio::test]
pub(crate) async fn persist_and_broadcast_proxy_capture_runtime_snapshot_uses_memory_overlay_without_sync_db_write()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let request_info = runtime_overlay_request_info();
    let invoke_id = "invoke-running";
    let occurred_at_owned = format_naive(Utc::now().naive_utc());
    let occurred_at = occurred_at_owned.as_str();
    let record = build_runtime_overlay_record(
        invoke_id,
        occurred_at,
        &request_info,
        RuntimeSnapshotSpec {
            account_id: 17,
            account_name: "pool-account-17",
            proxy_name: "jp-relay-01",
            attempt_count: 3,
            distinct_count: 2,
            timings: [22.0, 4.0, 330.0, 120.0],
        },
    );

    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, record)
        .await
        .expect("runtime snapshot should store in memory and broadcast");
    state.proxy_runtime_invocations.backdate_for_test(
        invoke_id,
        occurred_at,
        std::time::Duration::from_secs(2 * 60 * 60),
    );

    assert_initial_runtime_broadcast(&mut rx).await;

    assert_no_runtime_db_row(&state, invoke_id, occurred_at, "before flush").await;
    assert_runtime_overlay_filters(&state, invoke_id).await;
    assert_runtime_overlay_pagination(&state).await;

    state
        .sqlite_batch_writer
        .flush_buffered_for_test(&state.pool)
        .await;
    assert_no_runtime_db_row(&state, invoke_id, occurred_at, "after batch flush").await;

    let repeat_record = build_runtime_overlay_record(
        invoke_id,
        occurred_at,
        &request_info,
        RuntimeSnapshotSpec {
            account_id: 17,
            account_name: "pool-account-17",
            proxy_name: "jp-relay-01",
            attempt_count: 3,
            distinct_count: 2,
            timings: [24.0, 5.0, 340.0, 130.0],
        },
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, repeat_record)
        .await
        .expect("repeated runtime snapshot should only refresh memory");
    assert_no_runtime_db_row(&state, invoke_id, occurred_at, "after repeated snapshot").await;
    assert_runtime_account_switch(&state, invoke_id, occurred_at, &request_info).await;
    enqueue_terminal_runtime_overlay(&state, invoke_id, occurred_at).await;
    assert_terminal_runtime_flush(&state, invoke_id, occurred_at).await;
}

#[tokio::test]
pub(crate) async fn admitted_proxy_capture_snapshot_is_visible_before_body_parse_and_later_enriched()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "invoke-admitted-running";
    let occurred_at = "2026-03-17 18:14:34";

    let admitted_record = build_admitted_proxy_capture_runtime_snapshot(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        Some("203.0.113.42"),
        Some("sticky-from-header"),
        Some("pck-from-header"),
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, admitted_record)
        .await
        .expect("admitted snapshot should store in memory and broadcast");

    assert_admitted_broadcast(&mut rx).await;

    let enriched_request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-from-body".to_string()),
        sticky_key: Some("sticky-from-body".to_string()),
        prompt_cache_key_attribution_source: Some("request".to_string()),
        contains_encrypted_content: true,
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let enriched_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &enriched_request_info,
        Some("203.0.113.42"),
        Some("sticky-from-body"),
        Some("pck-from-body"),
        true,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        31.0,
        7.0,
        0.0,
        0.0,
    );
    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, enriched_record)
        .await
        .expect("body-parsed snapshot should enrich the same runtime row");

    assert_enriched_broadcast(&mut rx).await;
    assert_single_enriched_runtime_row(&state, invoke_id, occurred_at).await;
}

use super::*;
