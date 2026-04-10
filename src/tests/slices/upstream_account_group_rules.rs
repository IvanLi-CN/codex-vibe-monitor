#[tokio::test]
async fn update_upstream_account_group_disabling_retry_clears_retry_count_and_deletes_empty_row() {
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
async fn update_upstream_account_clears_mother_without_promoting_group_peers() {
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
async fn create_oauth_login_session_persists_mother_flag() {
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
async fn create_oauth_login_session_relink_preserves_existing_metadata() {
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

fn test_stage_timings() -> StageTimings {
    StageTimings {
        t_total_ms: 0.0,
        t_req_read_ms: 0.0,
        t_req_parse_ms: 0.0,
        t_upstream_connect_ms: 0.0,
        t_upstream_ttfb_ms: 0.0,
        t_upstream_stream_ms: 0.0,
        t_resp_parse_ms: 0.0,
        t_persist_ms: 0.0,
    }
}

fn test_proxy_capture_record(invoke_id: &str, occurred_at: &str) -> ProxyCaptureRecord {
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
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        timings: test_stage_timings(),
    }
}

#[tokio::test]
async fn persist_and_broadcast_proxy_capture_runtime_snapshot_emits_queryable_running_record() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        prompt_cache_key: Some("pck-running".to_string()),
        requested_service_tier: Some("priority".to_string()),
        reasoning_effort: Some("high".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-running";
    let occurred_at = "2026-03-17 18:13:34";
    let record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        Some("pck-running"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api-keys.vendor.invalid"),
        Some("jp-relay-01"),
        Some(3),
        Some(2),
        None,
        Some("gzip"),
        22.0,
        4.0,
        330.0,
        120.0,
    );

    persist_and_broadcast_proxy_capture_runtime_snapshot(&state, record)
        .await
        .expect("runtime snapshot should persist and broadcast");

    let payload = rx
        .recv()
        .await
        .expect("runtime snapshot payload should arrive");
    let broadcast_record = match payload {
        BroadcastPayload::Records { records } => {
            assert_eq!(records.len(), 1);
            records.into_iter().next().expect("single running record")
        }
        other => panic!("expected records payload, got {other:?}"),
    };

    assert!(
        broadcast_record.id > 0,
        "running snapshot should use persisted row id"
    );
    assert_eq!(broadcast_record.status.as_deref(), Some("running"));
    assert_eq!(broadcast_record.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(broadcast_record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(
        broadcast_record.proxy_display_name.as_deref(),
        Some("jp-relay-01")
    );
    assert_eq!(broadcast_record.upstream_account_id, Some(17));
    assert_eq!(
        broadcast_record.upstream_account_name.as_deref(),
        Some("pool-account-17")
    );
    assert_eq!(
        broadcast_record.response_content_encoding.as_deref(),
        Some("gzip")
    );
    assert_eq!(broadcast_record.pool_attempt_count, Some(3));
    assert_eq!(broadcast_record.pool_distinct_account_count, Some(2));
    assert_eq!(broadcast_record.pool_attempt_terminal_reason, None);
    assert_eq!(
        broadcast_record.prompt_cache_key.as_deref(),
        Some("pck-running")
    );
    assert_eq!(broadcast_record.billing_service_tier, None);
    assert_eq!(broadcast_record.t_total_ms, None);
    assert_eq!(broadcast_record.t_req_read_ms, Some(22.0));
    assert_eq!(broadcast_record.t_req_parse_ms, Some(4.0));
    assert_eq!(broadcast_record.t_upstream_connect_ms, Some(330.0));
    assert_eq!(broadcast_record.t_upstream_ttfb_ms, Some(120.0));

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            request_id: Some(invoke_id.to_string()),
            page_size: Some(1),
            ..Default::default()
        }),
    )
    .await
    .expect("running invocation should be queryable immediately");

    assert_eq!(response.total, 1);
    assert_eq!(response.records.len(), 1);
    assert_eq!(response.records[0].id, broadcast_record.id);
    assert_eq!(response.records[0].status.as_deref(), Some("running"));
    assert_eq!(response.records[0].pool_attempt_count, Some(3));
    assert_eq!(response.records[0].pool_distinct_account_count, Some(2));
    assert_eq!(response.records[0].pool_attempt_terminal_reason, None);
}

#[tokio::test]
async fn persist_proxy_capture_record_finalizes_existing_running_row_in_place() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: false,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-runtime-broadcast";
    let occurred_at = "2026-03-17 18:13:34";
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.88"),
        None,
        None,
        false,
        None,
        None,
        None,
        None,
        Some("edge-runtime"),
        None,
        None,
        None,
        None,
        12.0,
        3.0,
        99.0,
        0.0,
    );

    let running = persist_proxy_capture_runtime_record(&state.pool, running_record)
        .await
        .expect("persist running record")
        .expect("running record should be inserted");
    assert!(running.id > 0);
    assert_eq!(running.status.as_deref(), Some("running"));

    let finalized = persist_proxy_capture_record(
        &state.pool,
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("finalize record")
    .expect("terminal update should reuse running row");

    assert_eq!(finalized.id, running.id);
    assert_eq!(finalized.status.as_deref(), Some("success"));
    let duplicate_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE invoke_id = ?1 AND occurred_at = ?2
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .fetch_one(&state.pool)
    .await
    .expect("count invocation rows");
    assert_eq!(duplicate_count, 1);
}

#[tokio::test]
async fn persist_proxy_capture_record_repairs_proxy_interrupted_recovery_row_with_terminal_result()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let request_info = RequestCaptureInfo {
        model: Some("gpt-5.4".to_string()),
        is_stream: true,
        ..RequestCaptureInfo::default()
    };
    let invoke_id = "invoke-repair-interrupted-terminal";
    let occurred_at = "2026-03-17 18:13:35";
    let running_record = build_running_proxy_capture_record(
        invoke_id,
        occurred_at,
        ProxyCaptureTarget::Responses,
        &request_info,
        Some("198.51.100.89"),
        Some("sticky-repair-interrupted"),
        Some("pck-repair-interrupted"),
        true,
        Some(17),
        Some("pool-account-17"),
        Some("api_key_codex"),
        Some("api.openai.com"),
        None,
        Some(1),
        Some(1),
        None,
        None,
        12.0,
        3.0,
        99.0,
        120.0,
    );

    let running = persist_proxy_capture_runtime_record(&state.pool, running_record)
        .await
        .expect("persist running record")
        .expect("running record should be inserted");
    sqlx::query(
        r#"
        UPDATE codex_invocations
        SET status = ?2, failure_kind = ?3
        WHERE id = ?1
        "#,
    )
    .bind(running.id)
    .bind(INVOCATION_STATUS_INTERRUPTED)
    .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    .execute(&state.pool)
    .await
    .expect("mark running row as proxy-interrupted recovery artifact");

    let recovery_row = sqlx::query_as::<_, (String, Option<String>)>(
        r#"
        SELECT status, failure_kind
        FROM codex_invocations
        WHERE id = ?1
        "#,
    )
    .bind(running.id)
    .fetch_one(&state.pool)
    .await
    .expect("load recovery artifact row");
    assert_eq!(recovery_row.0, INVOCATION_STATUS_INTERRUPTED);
    assert_eq!(
        recovery_row.1.as_deref(),
        Some(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    );

    let repairable_count = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE id = ?1
          AND (
                LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
                OR (
                    LOWER(TRIM(COALESCE(status, ''))) = ?2
                    AND LOWER(TRIM(COALESCE(failure_kind, ''))) = ?3
                )
          )
        "#,
    )
    .bind(running.id)
    .bind(INVOCATION_STATUS_INTERRUPTED)
    .bind(PROXY_FAILURE_INVOCATION_INTERRUPTED)
    .fetch_one(&state.pool)
    .await
    .expect("count repairable rows");
    assert_eq!(repairable_count, 1);

    let mut tx = state.pool.begin().await.expect("begin verification tx");
    let existing = load_persisted_invocation_identity_tx(tx.as_mut(), invoke_id, occurred_at)
        .await
        .expect("load persisted invocation identity")
        .expect("persisted invocation identity should exist");
    assert!(
        invocation_status_is_recoverable_proxy_interrupted(
            existing.status.as_deref(),
            existing.failure_kind.as_deref(),
        ),
        "persisted invocation identity should be recognized as repairable interrupted state",
    );
    tx.commit().await.expect("commit verification tx");

    let finalized = persist_proxy_capture_record(
        &state.pool,
        Instant::now(),
        test_proxy_capture_record(invoke_id, occurred_at),
    )
    .await
    .expect("finalize record")
    .expect("terminal update should repair proxy-interrupted recovery row");

    assert_eq!(finalized.id, running.id);
    assert_eq!(finalized.status.as_deref(), Some("success"));
    assert_eq!(finalized.failure_kind, None);
}

async fn seed_quota_snapshot(pool: &SqlitePool, captured_at: &str) {
    sqlx::query(
        r#"
        INSERT INTO codex_quota_snapshots (
            captured_at,
            amount_limit,
            used_amount,
            remaining_amount,
            period,
            period_reset_time,
            expire_time,
            is_active,
            total_cost,
            total_requests,
            total_tokens,
            last_request_time,
            billing_type,
            remaining_count,
            used_count,
            sub_type_name
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
    )
    .bind(captured_at)
    .bind(Some(100.0))
    .bind(Some(10.0))
    .bind(Some(90.0))
    .bind(Some("monthly"))
    .bind(Some("2026-03-01 00:00:00"))
    .bind(None::<String>)
    .bind(1_i64)
    .bind(10.0)
    .bind(9_i64)
    .bind(150_i64)
    .bind(Some(captured_at))
    .bind(Some("prepaid"))
    .bind(Some(91_i64))
    .bind(Some(9_i64))
    .bind(Some("unit"))
    .execute(pool)
    .await
    .expect("seed quota snapshot");
}

async fn seed_forward_proxy_attempt_at(
    pool: &SqlitePool,
    proxy_key: &str,
    occurred_at: DateTime<Utc>,
    is_success: bool,
) {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            proxy_key,
            occurred_at,
            is_success,
            latency_ms,
            failure_kind,
            is_probe
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(proxy_key)
    .bind(occurred_at.format("%Y-%m-%d %H:%M:%S").to_string())
    .bind(is_success as i64)
    .bind(if is_success { Some(120.0) } else { None })
    .bind(if is_success {
        None::<String>
    } else {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR.to_string())
    })
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("seed forward proxy attempt");
}

async fn seed_forward_proxy_hourly_bucket_at(
    pool: &SqlitePool,
    proxy_key: &str,
    bucket_start_epoch: i64,
    success_count: i64,
    failure_count: i64,
) {
    let attempts = success_count + failure_count;
    let latency_sample_count = success_count.max(0);
    let latency_sum_ms = if latency_sample_count > 0 {
        latency_sample_count as f64 * 120.0
    } else {
        0.0
    };
    let latency_max_ms = if latency_sample_count > 0 { 120.0 } else { 0.0 };
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempt_hourly (
            proxy_key,
            bucket_start_epoch,
            attempts,
            success_count,
            failure_count,
            latency_sample_count,
            latency_sum_ms,
            latency_max_ms,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))
        ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
            attempts = excluded.attempts,
            success_count = excluded.success_count,
            failure_count = excluded.failure_count,
            latency_sample_count = excluded.latency_sample_count,
            latency_sum_ms = excluded.latency_sum_ms,
            latency_max_ms = excluded.latency_max_ms,
            updated_at = datetime('now')
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .bind(attempts)
    .bind(success_count)
    .bind(failure_count)
    .bind(latency_sample_count)
    .bind(latency_sum_ms)
    .bind(latency_max_ms)
    .execute(pool)
    .await
    .expect("seed forward proxy hourly bucket");
}

#[allow(clippy::too_many_arguments)]
async fn seed_forward_proxy_weight_bucket_at(
    pool: &SqlitePool,
    proxy_key: &str,
    bucket_start_epoch: i64,
    sample_count: i64,
    min_weight: f64,
    max_weight: f64,
    avg_weight: f64,
    last_weight: f64,
) {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_weight_hourly (
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
        ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
            sample_count = excluded.sample_count,
            min_weight = excluded.min_weight,
            max_weight = excluded.max_weight,
            avg_weight = excluded.avg_weight,
            last_weight = excluded.last_weight,
            updated_at = datetime('now')
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .bind(sample_count)
    .bind(min_weight)
    .bind(max_weight)
    .bind(avg_weight)
    .bind(last_weight)
    .execute(pool)
    .await
    .expect("seed forward proxy weight bucket");
}

async fn drain_broadcast_messages(rx: &mut broadcast::Receiver<BroadcastPayload>) {
    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => break,
        }
    }
}

async fn spawn_test_forward_proxy_status(status: StatusCode) -> (String, JoinHandle<()>) {
    let app = Router::new().fallback(any(move || async move {
        (
            status,
            Json(json!({
                "status": status.as_u16(),
            })),
        )
    }));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind forward proxy status test server");
    let addr = listener
        .local_addr()
        .expect("forward proxy status test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("forward proxy status test server should run");
    });

    (format!("http://{addr}"), handle)
}

async fn spawn_test_blocking_forward_proxy_status(
    status: StatusCode,
    request_started: Arc<Notify>,
    release_request: Arc<Notify>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().fallback(any(move || {
        let request_started = request_started.clone();
        let release_request = release_request.clone();
        async move {
            request_started.notify_waiters();
            release_request.notified().await;
            (
                status,
                Json(json!({
                    "status": status.as_u16(),
                })),
            )
        }
    }));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind blocking forward proxy status test server");
    let addr = listener
        .local_addr()
        .expect("blocking forward proxy status test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("blocking forward proxy status test server should run");
    });

    (format!("http://{addr}"), handle)
}

async fn spawn_test_subscription_source(body: String) -> (String, JoinHandle<()>) {
    let body = Arc::new(body);
    let app = Router::new().route(
        "/subscription",
        get({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move { (StatusCode::OK, body.as_str().to_string()) }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind subscription source test server");
    let addr = listener
        .local_addr()
        .expect("subscription source test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("subscription source test server should run");
    });

    (format!("http://{addr}/subscription"), handle)
}

async fn count_forward_proxy_probe_attempts(
    pool: &SqlitePool,
    proxy_key: &str,
    success: Option<bool>,
) -> i64 {
    let query = match success {
        Some(true) => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success != 0"
        }
        Some(false) => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success = 0"
        }
        None => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0"
        }
    };
    sqlx::query_scalar(query)
        .bind(proxy_key)
        .fetch_one(pool)
        .await
        .expect("count forward proxy probe attempts")
}

async fn wait_for_forward_proxy_probe_attempts(
    pool: &SqlitePool,
    proxy_key: &str,
    expected_min_count: i64,
) {
    let started = Instant::now();
    loop {
        let count = count_forward_proxy_probe_attempts(pool, proxy_key, None).await;
        if count >= expected_min_count {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting forward proxy probe attempts for {proxy_key}; expected at least {expected_min_count}, got {count}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn count_request_forward_proxy_attempts(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM forward_proxy_attempts WHERE is_probe = 0")
        .fetch_one(pool)
        .await
        .expect("count request forward proxy attempts")
}

async fn count_request_forward_proxy_attempts_with_failure_kind(
    pool: &SqlitePool,
    failure_kind: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM forward_proxy_attempts WHERE is_probe = 0 AND failure_kind = ?1",
    )
    .bind(failure_kind)
    .fetch_one(pool)
    .await
    .expect("count request forward proxy attempts by failure kind")
}

async fn count_pool_upstream_request_attempts(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
        .fetch_one(pool)
        .await
        .expect("count pool upstream request attempts")
}

async fn wait_for_pool_upstream_request_attempts(pool: &SqlitePool, expected_min_count: i64) {
    let started = Instant::now();
    loop {
        let count = count_pool_upstream_request_attempts(pool).await;
        if count >= expected_min_count {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting pool upstream attempts; expected at least {expected_min_count}, got {count}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn latest_request_forward_proxy_attempt_latency_ms(pool: &SqlitePool) -> Option<f64> {
    sqlx::query_scalar::<_, Option<f64>>(
        "SELECT latency_ms FROM forward_proxy_attempts WHERE is_probe = 0 ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .expect("fetch latest request forward proxy latency")
    .flatten()
}

async fn count_codex_invocations(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(pool)
        .await
        .expect("count codex invocations")
}

async fn count_in_flight_codex_invocations(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM codex_invocations
        WHERE LOWER(TRIM(COALESCE(status, ''))) IN ('running', 'pending')
        "#,
    )
    .fetch_one(pool)
    .await
    .expect("count in-flight codex invocations")
}

async fn wait_for_codex_invocations(pool: &SqlitePool, expected_min_count: i64) {
    let started = Instant::now();
    loop {
        let count = count_codex_invocations(pool).await;
        let in_flight = count_in_flight_codex_invocations(pool).await;
        if count >= expected_min_count && in_flight == 0 {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting for codex invocations; expected at least {expected_min_count}, got {count}, in_flight={in_flight}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[derive(Clone)]
struct Retry429EchoState {
    rate_limit_attempts: usize,
    attempts: Arc<AtomicUsize>,
    seen_bodies: Arc<StdMutex<Vec<String>>>,
    retry_after: Option<HeaderValue>,
}

async fn retrying_echo_upstream(
    State(state): State<Retry429EchoState>,
    method: Method,
    uri: Uri,
    body: String,
) -> Response {
    state
        .seen_bodies
        .lock()
        .expect("lock retrying echo bodies")
        .push(body.clone());
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;

    if attempt <= state.rate_limit_attempts {
        let mut headers = HeaderMap::new();
        if let Some(retry_after) = state.retry_after.clone() {
            headers.insert(http_header::RETRY_AFTER, retry_after);
        }
        headers.insert(
            http_header::HeaderName::from_static("x-upstream-attempt"),
            HeaderValue::from_str(&attempt.to_string()).expect("valid attempt header"),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({
                "error": "rate limited",
                "attempt": attempt,
                "method": method.as_str(),
                "query": uri.query().unwrap_or_default(),
                "body": body,
            })),
        )
            .into_response();
    }

    (
        StatusCode::CREATED,
        Json(json!({
            "attempt": attempt,
            "method": method.as_str(),
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
            "body": body,
        })),
    )
        .into_response()
}

async fn spawn_retrying_echo_upstream(
    rate_limit_attempts: usize,
    retry_after: Option<&str>,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<StdMutex<Vec<String>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen_bodies = Arc::new(StdMutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route("/v1/echo", any(retrying_echo_upstream))
        .with_state(Retry429EchoState {
            rate_limit_attempts,
            attempts: attempts.clone(),
            seen_bodies: seen_bodies.clone(),
            retry_after: retry_after
                .map(|value| HeaderValue::from_str(value).expect("valid retry-after header")),
        });

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retrying echo test server");
    let addr = listener
        .local_addr()
        .expect("retrying echo test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("retrying echo test server should run");
    });

    (format!("http://{addr}/"), attempts, seen_bodies, handle)
}

#[derive(Clone)]
struct Retry429CaptureState {
    rate_limit_attempts: usize,
    attempts: Arc<AtomicUsize>,
    seen_payloads: Arc<StdMutex<Vec<Value>>>,
    retry_after: Option<HeaderValue>,
}

async fn retrying_capture_upstream(
    State(state): State<Retry429CaptureState>,
    body: Bytes,
) -> Response {
    let payload: Value = serde_json::from_slice(&body).expect("decode retrying capture body");
    state
        .seen_payloads
        .lock()
        .expect("lock retrying capture payloads")
        .push(payload.clone());
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;

    if attempt <= state.rate_limit_attempts {
        let mut headers = HeaderMap::new();
        if let Some(retry_after) = state.retry_after.clone() {
            headers.insert(http_header::RETRY_AFTER, retry_after);
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({
                "error": "rate limited",
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_retry_success",
            "object": "response",
            "model": "gpt-5.3-codex",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "total_tokens": 15
            },
            "received": payload,
            "attempt": attempt,
        })),
    )
        .into_response()
}

async fn spawn_retrying_capture_upstream(
    rate_limit_attempts: usize,
    retry_after: Option<&str>,
) -> (
    String,
    Arc<AtomicUsize>,
    Arc<StdMutex<Vec<Value>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen_payloads = Arc::new(StdMutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route("/v1/chat/completions", post(retrying_capture_upstream))
        .route("/v1/responses", post(retrying_capture_upstream))
        .with_state(Retry429CaptureState {
            rate_limit_attempts,
            attempts: attempts.clone(),
            seen_payloads: seen_payloads.clone(),
            retry_after: retry_after
                .map(|value| HeaderValue::from_str(value).expect("valid retry-after header")),
        });

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retrying capture test server");
    let addr = listener
        .local_addr()
        .expect("retrying capture test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("retrying capture test server should run");
    });

    (format!("http://{addr}/"), attempts, seen_payloads, handle)
}

#[derive(Clone)]
struct Retry429ModelsState {
    rate_limit_attempts: usize,
    attempts: Arc<AtomicUsize>,
    retry_after: Option<HeaderValue>,
}

async fn retrying_models_upstream(State(state): State<Retry429ModelsState>) -> Response {
    let attempt = state.attempts.fetch_add(1, Ordering::SeqCst) + 1;

    if attempt <= state.rate_limit_attempts {
        let mut headers = HeaderMap::new();
        if let Some(retry_after) = state.retry_after.clone() {
            headers.insert(http_header::RETRY_AFTER, retry_after);
        }
        headers.insert(
            http_header::HeaderName::from_static("x-upstream-attempt"),
            HeaderValue::from_str(&attempt.to_string()).expect("valid attempt header"),
        );
        return (
            StatusCode::TOO_MANY_REQUESTS,
            headers,
            Json(json!({
                "error": "rate limited",
                "attempt": attempt,
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "object": "list",
            "data": [
                {
                    "id": "upstream-model-after-retry",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345680
                }
            ]
        })),
    )
        .into_response()
}

async fn spawn_retrying_models_upstream(
    rate_limit_attempts: usize,
    retry_after: Option<&str>,
) -> (String, Arc<AtomicUsize>, JoinHandle<()>) {
    let attempts = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/v1/models", get(retrying_models_upstream))
        .with_state(Retry429ModelsState {
            rate_limit_attempts,
            attempts: attempts.clone(),
            retry_after: retry_after
                .map(|value| HeaderValue::from_str(value).expect("valid retry-after header")),
        });

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind retrying models test server");
    let addr = listener
        .local_addr()
        .expect("retrying models test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("retrying models test server should run");
    });

    (format!("http://{addr}/"), attempts, handle)
}

async fn read_forward_proxy_runtime_weight(pool: &SqlitePool, proxy_key: &str) -> Option<f64> {
    sqlx::query_scalar::<_, f64>("SELECT weight FROM forward_proxy_runtime WHERE proxy_key = ?1")
        .bind(proxy_key)
        .fetch_optional(pool)
        .await
        .expect("read forward proxy runtime weight")
}

async fn test_upstream_echo(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let auth = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let host_header = headers
        .get(http_header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let connection_seen = headers.contains_key(http_header::CONNECTION);
    let x_foo_seen = headers.contains_key(http_header::HeaderName::from_static("x-foo"));
    let x_forwarded_for_seen =
        headers.contains_key(http_header::HeaderName::from_static("x-forwarded-for"));
    let forwarded_seen = headers.contains_key(http_header::HeaderName::from_static("forwarded"));
    let via_seen = headers.contains_key(http_header::HeaderName::from_static("via"));
    let accept_encoding = headers
        .get(http_header::ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        http_header::HeaderName::from_static("x-upstream"),
        HeaderValue::from_static("ok"),
    );
    response_headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("x-upstream-hop"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("x-upstream-hop"),
        HeaderValue::from_static("should-be-filtered"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("via"),
        HeaderValue::from_static("1.1 upstream-proxy"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=192.0.2.1;proto=https;host=api.example.com"),
    );

    (
        StatusCode::CREATED,
        response_headers,
        Json(json!({
            "method": method.as_str(),
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
            "authorization": auth,
            "hostHeader": host_header,
            "connectionSeen": connection_seen,
            "xFooSeen": x_foo_seen,
            "xForwardedForSeen": x_forwarded_for_seen,
            "forwardedSeen": forwarded_seen,
            "viaSeen": via_seen,
            "acceptEncoding": accept_encoding,
            "body": body,
        })),
    )
}

async fn test_upstream_stream() -> impl IntoResponse {
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")),
        Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")),
    ]);
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_stream_first_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-first-chunk-error")),
                    1,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_stream_mid_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((Ok::<Bytes, io::Error>(Bytes::from_static(b"chunk-a")), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-mid-stream-error")),
                    2,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_429_mid_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((
                Ok::<Bytes, io::Error>(Bytes::from_static(
                    br#"{"error":{"message":"rate limited"}}"#,
                )),
                1,
            )),
            1 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-429-mid-stream-error")),
                    2,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::TOO_MANY_REQUESTS,
        [(http_header::RETRY_AFTER, HeaderValue::from_static("0"))],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_slow_stream() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")), 2))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_slow_first_chunk() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")), 1))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_hang() -> impl IntoResponse {
    tokio::time::sleep(Duration::from_secs(2)).await;
    StatusCode::NO_CONTENT
}

async fn test_upstream_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("/v1/echo?from=redirect"),
        )],
        Body::empty(),
    )
}

async fn test_upstream_external_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("https://example.org/outside"),
        )],
        Body::empty(),
    )
}

async fn test_upstream_chat_external_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("https://example.org/outside"),
        )],
        Body::empty(),
    )
}

async fn test_upstream_responses_gzip_stream() -> impl IntoResponse {
    let payload = [
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}\n\n",
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    (
        StatusCode::OK,
        [
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            ),
            (
                http_header::CONTENT_ENCODING,
                HeaderValue::from_static("gzip"),
            ),
        ],
        Body::from(compressed),
    )
}

fn less_compressible_test_string(target_len: usize) -> String {
    use std::fmt::Write as _;

    let mut text = String::with_capacity(target_len);
    let mut value = 0x1234_abcd_u32;
    while text.len() < target_len {
        value = value.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let _ = write!(&mut text, "{value:08x}");
    }
    text.truncate(target_len);
    text
}

async fn test_upstream_responses_gzip_stream_without_event_stream_header() -> impl IntoResponse {
    let payload = [
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test_no_ct\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test_no_ct\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":19,\"output_tokens\":6,\"total_tokens\":25}}}\n\n",
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write gzip payload without event-stream header");
    let compressed = encoder
        .finish()
        .expect("finish gzip payload without event-stream header");

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_ENCODING,
            HeaderValue::from_static("gzip"),
        )],
        Body::from(compressed),
    )
}

async fn test_upstream_responses_large_gzip_stream_without_event_stream_header() -> impl IntoResponse
{
    let large_delta = less_compressible_test_string(RAW_RESPONSE_PREVIEW_LIMIT * 8);
    let payload = [
        "event: response.created\n".to_string(),
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test_no_ct_large\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n".to_string(),
        format!(
            "event: response.output_text.delta\n\
             data: {}\n\n",
            serde_json::to_string(&json!({
                "type": "response.output_text.delta",
                "delta": large_delta,
            }))
            .expect("serialize large gzip delta payload")
        ),
        "event: response.completed\n".to_string(),
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test_no_ct_large\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":23,\"output_tokens\":7,\"total_tokens\":30}}}\n\n".to_string(),
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write large gzip payload without event-stream header");
    let compressed = encoder
        .finish()
        .expect("finish large gzip payload without event-stream header");
    assert!(
        compressed.len() > RAW_RESPONSE_PREVIEW_LIMIT,
        "large gzip payload should exceed preview cap"
    );

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_ENCODING,
            HeaderValue::from_static("gzip"),
        )],
        Body::from(compressed),
    )
}

async fn test_upstream_responses_slow_success_stream() -> impl IntoResponse {
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_slow_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15}}}\n\n",
    );
    let chunks = stream::unfold(0usize, move |state| async move {
        match state {
            0 => Some((Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((
                    Ok::<_, Infallible>(Bytes::from_static(second.as_bytes())),
                    2,
                ))
            }
            _ => None,
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_responses_large_stream() -> impl IntoResponse {
    let large_delta = "x".repeat(RAW_RESPONSE_PREVIEW_LIMIT + 8 * 1024);
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_large_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = format!(
        "event: response.output_text.delta\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.output_text.delta",
            "delta": large_delta,
        }))
        .expect("serialize large delta payload")
    );
    let third = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_large_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":42,\"output_tokens\":13,\"total_tokens\":55}}}\n\n",
    );
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())),
        Ok::<_, Infallible>(Bytes::from(second)),
        Ok::<_, Infallible>(Bytes::from_static(third.as_bytes())),
    ]);

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_responses_large_terminal_stream() -> impl IntoResponse {
    let large_terminal_text =
        less_compressible_test_string(STREAM_RESPONSE_LINE_BUFFER_LIMIT + 64 * 1024);
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_large_terminal_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = format!(
        "event: response.completed\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.completed",
            "response": {
                "id": "resp_large_terminal_test",
                "model": "gpt-5.4",
                "status": "completed",
                "service_tier": "priority",
                "usage": {
                    "input_tokens": 77,
                    "output_tokens": 19,
                    "total_tokens": 96,
                },
                "output": [{
                    "type": "output_text",
                    "text": large_terminal_text,
                }],
            },
        }))
        .expect("serialize large terminal payload")
    );
    assert!(
        second.len() > STREAM_RESPONSE_LINE_BUFFER_LIMIT,
        "terminal SSE event should exceed the hot-path line buffer limit"
    );
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())),
        Ok::<_, Infallible>(Bytes::from(second)),
    ]);

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_responses_oversized_delta_stream() -> impl IntoResponse {
    let oversized_delta =
        less_compressible_test_string(STREAM_RESPONSE_LINE_BUFFER_LIMIT + 64 * 1024);
    let first = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_oversized_delta_test\",\"model\":\"gpt-5.4\",\"status\":\"in_progress\"}}\n\n",
    );
    let second = format!(
        "event: response.output_text.delta\n\
         data: {}\n\n",
        serde_json::to_string(&json!({
            "type": "response.output_text.delta",
            "delta": oversized_delta,
        }))
        .expect("serialize oversized delta payload")
    );
    assert!(
        second.len() > STREAM_RESPONSE_LINE_BUFFER_LIMIT,
        "oversized SSE delta should exceed the hot-path line buffer limit"
    );
    let third = concat!(
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_oversized_delta_test\",\"model\":\"gpt-5.4\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":61,\"output_tokens\":17,\"total_tokens\":78}}}\n\n",
    );
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(first.as_bytes())),
        Ok::<_, Infallible>(Bytes::from(second)),
        Ok::<_, Infallible>(Bytes::from_static(third.as_bytes())),
    ]);

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_responses_failed_stream() -> impl IntoResponse {
    let payload = [
        "event: response.created
",
        r#"data: {"type":"response.created","response":{"id":"resp_fail_test","model":"gpt-5.4","status":"in_progress"}}"#,
        "

",
        r#"data: {"type":"error","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}"#,
        "

",
        "event: response.failed
",
        r#"data: {"type":"response.failed","response":{"id":"resp_fail_test","model":"gpt-5.4","status":"failed","error":{"code":"server_error","message":"An error occurred while processing your request. Please include the request ID 060a328d-5cb6-433c-9025-1da2d9c632f1 in your message."}}}"#,
        "

",
    ]
    .concat();

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from(payload),
    )
}

async fn test_upstream_responses_large_json_error() -> impl IntoResponse {
    let message = format!(
        "validation failed: {} tail-marker",
        "x".repeat(RAW_RESPONSE_PREVIEW_LIMIT + 8 * 1024)
    );
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": {
                "code": "invalid_request_error",
                "message": message
            }
        })),
    )
        .into_response()
}

async fn test_upstream_responses_large_prefixed_json_error() -> impl IntoResponse {
    let oversized_detail =
        less_compressible_test_string(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 8 * 1024);
    let body = format!(
        r#"{{"error":{{"code":"invalid_request_error","message":"prefix metadata should survive"}},"service_tier":"priority","detail":"{}"}}"#,
        oversized_detail
    );
    (
        StatusCode::BAD_REQUEST,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from(body),
    )
}

async fn test_upstream_responses(uri: Uri) -> Response {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=response_failed"))
    {
        test_upstream_responses_failed_stream()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=json-error"))
    {
        test_upstream_responses_large_json_error()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=large-prefixed-json-error"))
    {
        test_upstream_responses_large_prefixed_json_error()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-success"))
    {
        test_upstream_responses_slow_success_stream()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=large-stream"))
    {
        test_upstream_responses_large_stream().await.into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=large-terminal-stream"))
    {
        test_upstream_responses_large_terminal_stream()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=oversized-delta-stream"))
    {
        test_upstream_responses_oversized_delta_stream()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=gzip-large-no-content-type"))
    {
        test_upstream_responses_large_gzip_stream_without_event_stream_header()
            .await
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=gzip-no-content-type"))
    {
        test_upstream_responses_gzip_stream_without_event_stream_header()
            .await
            .into_response()
    } else if uri.query().is_some_and(|query| query.contains("mode=gzip")) {
        test_upstream_responses_gzip_stream().await.into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
        (
            StatusCode::OK,
            Json(json!({
                "id": "resp_delayed_test",
                "object": "response",
                "model": "gpt-5.3-codex",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 3,
                    "total_tokens": 15
                }
            })),
        )
            .into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=no-content"))
    {
        StatusCode::NO_CONTENT.into_response()
    } else {
        test_upstream_stream_mid_error().await.into_response()
    }
}

async fn test_upstream_responses_compact(uri: Uri) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=large-json"))
    {
        return (
            StatusCode::OK,
            Json(json!({
                "id": "resp_compact_large",
                "object": "response.compaction",
                "service_tier": "priority",
                "output": [
                    {
                        "id": "cmp_large_001",
                        "type": "compaction",
                        "encrypted_content": "z".repeat(BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES + 8 * 1024)
                    }
                ],
                "usage": {
                    "input_tokens": 201,
                    "output_tokens": 99,
                    "total_tokens": 300
                }
            })),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_compact_test",
            "object": "response.compaction",
            "output": [
                {
                    "id": "cmp_001",
                    "type": "compaction",
                    "encrypted_content": "encrypted-summary"
                }
            ],
            "usage": {
                "input_tokens": 139,
                "input_tokens_details": {
                    "cached_tokens": 11
                },
                "output_tokens": 438,
                "output_tokens_details": {
                    "reasoning_tokens": 64
                },
                "total_tokens": 577
            }
        })),
    )
        .into_response()
}

async fn test_upstream_models(uri: Uri) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=error"))
    {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": "upstream model list unavailable"
            })),
        )
            .into_response();
    }

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-body"))
    {
        let chunked = stream::unfold(0u8, |state| async move {
            match state {
                0 => Some((
                    Ok::<Bytes, Infallible>(Bytes::from_static(br#"{"object":"list","data":["#)),
                    1,
                )),
                1 => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Some((
                        Ok::<Bytes, Infallible>(Bytes::from_static(
                            br#"{"id":"slow-model","object":"model"}]}"#,
                        )),
                        2,
                    ))
                }
                _ => None,
            }
        });
        return (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            Body::from_stream(chunked),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "object": "list",
            "data": [
                {
                    "id": "upstream-model-a",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345678
                },
                {
                    "id": "gpt-5.2-codex",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345679
                }
            ]
        })),
    )
        .into_response()
}

async fn spawn_test_upstream() -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/echo", any(test_upstream_echo))
        .route("/v1/stream", any(test_upstream_stream))
        .route(
            "/v1/stream-first-error",
            any(test_upstream_stream_first_error),
        )
        .route("/v1/stream-mid-error", any(test_upstream_stream_mid_error))
        .route("/v1/429-mid-error", any(test_upstream_429_mid_error))
        .route("/v1/slow-stream", any(test_upstream_slow_stream))
        .route("/v1/slow-first-chunk", any(test_upstream_slow_first_chunk))
        .route("/v1/hang", any(test_upstream_hang))
        .route("/v1/models", get(test_upstream_models))
        .route("/v1/redirect", any(test_upstream_redirect))
        .route(
            "/v1/redirect-external",
            any(test_upstream_external_redirect),
        )
        .route(
            "/v1/chat/completions",
            any(test_upstream_chat_external_redirect),
        )
        .route("/v1/responses", any(test_upstream_responses))
        .route(
            "/v1/responses/compact",
            post(test_upstream_responses_compact),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream test server");
    let addr = listener.local_addr().expect("upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("upstream test server should run");
    });

    (format!("http://{addr}/"), handle)
}

async fn spawn_test_upstream_with_prefix(prefix: &str) -> (String, JoinHandle<()>) {
    let echo_path = format!("{prefix}/v1/echo");
    let redirect_path = format!("{prefix}/v1/redirect");
    let redirect_location = HeaderValue::from_str(&format!("{prefix}/v1/echo?from=redirect"))
        .expect("valid redirect location");

    let app = Router::new()
        .route(&echo_path, any(test_upstream_echo))
        .route(
            &redirect_path,
            any({
                let redirect_location = redirect_location.clone();
                move || {
                    let redirect_location = redirect_location.clone();
                    async move {
                        (
                            StatusCode::TEMPORARY_REDIRECT,
                            [(http_header::LOCATION, redirect_location)],
                            Body::empty(),
                        )
                    }
                }
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind prefixed upstream test server");
    let addr = listener.local_addr().expect("prefixed upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("prefixed upstream test server should run");
    });

    (format!("http://{addr}{prefix}/"), handle)
}

async fn test_upstream_capture_target_echo(
    State(captured): State<Arc<Mutex<Vec<Value>>>>,
    uri: Uri,
    body: Bytes,
) -> Response {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
    captured.lock().await.push(payload.clone());
    let response_payload = json!({
        "id": "resp_test",
        "object": "response",
        "model": "gpt-5.3-codex",
        "service_tier": "priority",
        "usage": {
            "input_tokens": 12,
            "output_tokens": 3,
            "total_tokens": 15
        },
        "received": payload,
    });

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-first-chunk"))
    {
        return chunked_json_response_with_delayed_first_chunk(
            response_payload,
            Duration::from_millis(250),
        );
    }
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-stream-end"))
    {
        return chunked_json_response_with_delayed_final_chunk(
            response_payload,
            Duration::from_millis(400),
        );
    }

    (StatusCode::OK, Json(response_payload)).into_response()
}

async fn test_upstream_capture_target_compact_echo(
    State(captured): State<Arc<Mutex<Vec<Value>>>>,
    uri: Uri,
    body: Bytes,
) -> Response {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
    captured.lock().await.push(payload.clone());
    let response_payload = json!({
        "id": "resp_compact_test",
        "object": "response.compaction",
        "output": [
            {
                "id": "cmp_001",
                "type": "compaction",
                "encrypted_content": "encrypted-summary"
            }
        ],
        "usage": {
            "input_tokens": 139,
            "input_tokens_details": {
                "cached_tokens": 11
            },
            "output_tokens": 438,
            "output_tokens_details": {
                "reasoning_tokens": 64
            },
            "total_tokens": 577
        },
        "received": payload,
    });

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-first-chunk"))
    {
        return chunked_json_response_with_delayed_first_chunk(
            response_payload,
            Duration::from_millis(250),
        );
    }
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-stream-end"))
    {
        return chunked_json_response_with_delayed_final_chunk(
            response_payload,
            Duration::from_millis(400),
        );
    }

    (StatusCode::OK, Json(response_payload)).into_response()
}

fn chunked_json_response_with_delayed_first_chunk(payload: Value, delay: Duration) -> Response {
    let response_bytes = serde_json::to_vec(&payload).expect("serialize streamed json response");
    let split_at = response_bytes
        .len()
        .saturating_div(2)
        .clamp(1, response_bytes.len() - 1);
    let first_chunk = Bytes::copy_from_slice(&response_bytes[..split_at]);
    let second_chunk = Bytes::copy_from_slice(&response_bytes[split_at..]);
    let chunked = stream::unfold(0u8, move |state| {
        let first_chunk = first_chunk.clone();
        let second_chunk = second_chunk.clone();
        async move {
            match state {
                0 => {
                    tokio::time::sleep(delay).await;
                    Some((Ok::<Bytes, Infallible>(first_chunk), 1))
                }
                1 => Some((Ok::<Bytes, Infallible>(second_chunk), 2)),
                _ => None,
            }
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from_stream(chunked),
    )
        .into_response()
}

fn chunked_json_response_with_delayed_final_chunk(payload: Value, delay: Duration) -> Response {
    let response_bytes = serde_json::to_vec(&payload).expect("serialize streamed json response");
    let split_at = response_bytes
        .len()
        .saturating_div(2)
        .clamp(1, response_bytes.len() - 1);
    let first_chunk = Bytes::copy_from_slice(&response_bytes[..split_at]);
    let second_chunk = Bytes::copy_from_slice(&response_bytes[split_at..]);
    let chunked = stream::unfold(0u8, move |state| {
        let first_chunk = first_chunk.clone();
        let second_chunk = second_chunk.clone();
        async move {
            match state {
                0 => Some((Ok::<Bytes, Infallible>(first_chunk), 1)),
                1 => {
                    tokio::time::sleep(delay).await;
                    Some((Ok::<Bytes, Infallible>(second_chunk), 2))
                }
                _ => None,
            }
        }
    });

    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )],
        Body::from_stream(chunked),
    )
        .into_response()
}

async fn spawn_capture_target_body_upstream() -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(test_upstream_capture_target_echo),
        )
        .route("/v1/responses", post(test_upstream_capture_target_echo))
        .route(
            "/v1/responses/compact",
            post(test_upstream_capture_target_compact_echo),
        )
        .with_state(captured.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind capture-target upstream test server");
    let addr = listener
        .local_addr()
        .expect("capture-target upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("capture-target upstream test server should run");
    });

    (format!("http://{addr}/"), captured, handle)
}

fn extract_model_ids(payload: &Value) -> Vec<String> {
    payload
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect()
}

#[test]
fn build_proxy_upstream_url_preserves_path_prefix_and_query() {
    let base = Url::parse("https://proxy.example.com/gateway").expect("valid base");
    let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
    let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
    assert_eq!(
        target.as_str(),
        "https://proxy.example.com/gateway/v1/models?limit=10"
    );
}

#[test]
fn build_proxy_upstream_url_supports_ipv6_literal_base() {
    let base = Url::parse("http://[::1]:8080/gateway/").expect("valid ipv6 base");
    let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
    let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
    assert_eq!(
        target.as_str(),
        "http://[::1]:8080/gateway/v1/models?limit=10"
    );
}

#[test]
fn path_has_forbidden_dot_segment_detects_plain_and_encoded_variants() {
    assert!(path_has_forbidden_dot_segment("/v1/../models"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e/models"));
    assert!(path_has_forbidden_dot_segment("/v1/.%2E/models"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%2fadmin"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%5cadmin"));
    assert!(path_has_forbidden_dot_segment("/v1/%252e%252e%252fadmin"));
    assert!(!path_has_forbidden_dot_segment("/v1/%2efoo/models"));
    assert!(!path_has_forbidden_dot_segment("/v1/models"));
}

#[test]
fn build_proxy_upstream_url_rejects_dot_segment_paths() {
    let base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let uri: Uri = "/v1/%2e%2e%2fadmin?scope=test"
        .parse()
        .expect("valid uri with dot segments");
    let err = build_proxy_upstream_url(&base, &uri).expect_err("dot segments should fail");
    assert!(
        err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED),
        "error should indicate forbidden dot segments: {err}"
    );
}

#[test]
fn has_invalid_percent_encoding_detects_malformed_sequences() {
    assert!(has_invalid_percent_encoding("/v1/%zz/models"));
    assert!(has_invalid_percent_encoding("/v1/%/models"));
    assert!(has_invalid_percent_encoding("/v1/%2/models"));
    assert!(!has_invalid_percent_encoding("/v1/%2F/models"));
    assert!(!has_invalid_percent_encoding("/v1/models"));
}

#[test]
fn should_transport_proxy_header_filters_only_transport_hop_by_hop_headers() {
    assert!(should_transport_proxy_header(&http_header::AUTHORIZATION));
    assert!(should_transport_proxy_header(&http_header::CONTENT_LENGTH));
    assert!(should_transport_proxy_header(&http_header::ACCEPT_ENCODING));
    assert!(!should_transport_proxy_header(&http_header::HOST));
    assert!(!should_transport_proxy_header(&http_header::CONNECTION));
    assert!(!should_transport_proxy_header(
        &http_header::TRANSFER_ENCODING
    ));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "forwarded"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "via"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-for"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-host"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-proto"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-forwarded-port"
    )));
    assert!(!should_transport_proxy_header(&HeaderName::from_static(
        "x-real-ip"
    )));
}

#[test]
fn connection_scoped_header_names_parses_connection_tokens() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("keep-alive, x-foo"),
    );
    headers.append(http_header::CONNECTION, HeaderValue::from_static("x-bar"));
    let names = connection_scoped_header_names(&headers);
    assert!(names.contains(&http_header::HeaderName::from_static("keep-alive")));
    assert!(names.contains(&http_header::HeaderName::from_static("x-foo")));
    assert!(names.contains(&http_header::HeaderName::from_static("x-bar")));
}

#[test]
fn request_may_have_body_uses_method_and_headers() {
    let empty = HeaderMap::new();
    assert!(!request_may_have_body(&Method::GET, &empty));
    assert!(request_may_have_body(&Method::POST, &empty));

    let mut with_length = HeaderMap::new();
    with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    assert!(!request_may_have_body(&Method::GET, &with_length));
    with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("10"));
    assert!(request_may_have_body(&Method::GET, &with_length));
}

#[test]
fn parse_cors_allowed_origins_normalizes_and_deduplicates() {
    let parsed = parse_cors_allowed_origins(
        "https://EXAMPLE.com:443, http://127.0.0.1:8080, https://example.com",
    )
    .expect("parse should succeed");
    assert_eq!(
        parsed,
        vec![
            "https://example.com".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ]
    );
}

#[test]
fn origin_allowed_accepts_loopback_and_configured_origins() {
    let configured = HashSet::from(["https://api.example.com".to_string()]);
    assert!(origin_allowed("http://127.0.0.1:60080", &configured));
    assert!(origin_allowed("https://api.example.com", &configured));
    assert!(!origin_allowed("https://evil.example.com", &configured));
}

#[test]
fn same_origin_settings_write_allows_missing_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_cross_site_without_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_matching_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:8080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_matching_origin_without_explicit_host_port() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    assert!(is_same_origin_settings_write(&headers));
}
