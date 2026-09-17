#[tokio::test]
async fn capture_target_pool_route_preserves_auth_failure_terminal_reason() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptFailureRow {
        status: String,
        http_status: Option<i64>,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_pool_http_failure_upstream(
        StatusCode::UNAUTHORIZED,
        Some("invalid_token"),
        "token expired",
    )
    .await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-attempts-auth-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read auth failure body");

    wait_for_codex_invocations(&state.pool, 1).await;

    let attempt_row = sqlx::query_as::<_, AttemptFailureRow>(
        r#"
        SELECT status, http_status, failure_kind
        FROM pool_upstream_request_attempts
        ORDER BY attempt_index ASC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load auth failure attempt row");
    assert_eq!(
        attempt_row.status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    );
    assert_eq!(
        attempt_row.http_status,
        Some(i64::from(StatusCode::UNAUTHORIZED.as_u16())),
    );
    assert_eq!(
        attempt_row.failure_kind.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
    );

    let row = sqlx::query_as::<_, PersistedPayloadRow>(
        r#"
        SELECT payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_one(&state.pool)
    .await
    .expect("load auth failure invocation payload");
    let payload: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("auth failure payload should be present"),
    )
    .expect("decode auth failure payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(1));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(1));
    assert_eq!(
        payload["poolAttemptTerminalReason"].as_str(),
        Some(PROXY_FAILURE_UPSTREAM_HTTP_AUTH),
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn pool_route_fails_over_on_unsupported_model_bad_request() {
    async fn unsupported_model_failover_upstream(
        attempts: Arc<StdMutex<HashMap<String, usize>>>,
        headers: HeaderMap,
    ) -> Response {
        let authorization = headers
            .get(http_header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let attempt = {
            let mut attempts = attempts.lock().expect("lock unsupported-model attempts");
            let entry = attempts.entry(authorization.clone()).or_insert(0);
            *entry += 1;
            *entry
        };
        if authorization == "Bearer upstream-primary" {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": {
                        "code": "unsupported_model",
                        "message": "unsupported model: gpt-5.5",
                    },
                    "attempt": attempt,
                })),
            )
                .into_response();
        }
        (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "authorization": authorization,
                "attempt": attempt,
            })),
        )
            .into_response()
    }

    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let app = Router::new().route(
        "/v1/responses",
        post({
            let attempts = attempts.clone();
            move |headers| unsupported_model_failover_upstream(attempts.clone(), headers)
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind unsupported-model failover upstream");
    let addr = listener
        .local_addr()
        .expect("unsupported-model failover upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("unsupported-model failover upstream should run");
    });
    let upstream_base = format!("http://{addr}");
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5.5","input":"hello","stickyKey":"sticky-unsupported-model-failover"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;

    assert_unsupported_model_failover(&state, &attempts, response, primary_id, secondary_id).await;

    upstream_handle.abort();
}

async fn assert_unsupported_model_failover(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    response: axum::response::Response,
    primary_id: i64,
    secondary_id: i64,
) {
    assert_eq!(response.status(), StatusCode::OK);
    let cvm_id = response
        .headers()
        .get(CVM_INVOKE_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("failover response should expose x-cvm-invoke-id");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read failover success body");
    let payload: Value = serde_json::from_slice(&body).expect("decode failover success body");
    assert_eq!(payload["authorization"], "Bearer upstream-secondary");
    wait_for_pool_attempt_terminal_observation(
        &state.pool,
        &cvm_id,
        1,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE,
        400,
    )
    .await;
    wait_for_pool_attempt_terminal_observation(
        &state.pool,
        &cvm_id,
        2,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS,
        200,
    )
    .await;
    let attempt_rows = sqlx::query_as::<_, (i64, Option<i64>, Option<String>)>("SELECT attempt_index, http_status, failure_kind FROM pool_upstream_request_attempts WHERE invoke_id = ?1 ORDER BY attempt_index ASC").bind(&cvm_id).fetch_all(&state.pool).await.expect("load unsupported-model attempt rows");
    assert_eq!(attempt_rows.len(), 2);
    assert_eq!(attempt_rows[0], (1, Some(400), attempt_rows[0].2.clone()));
    assert!(attempt_rows[0].2.is_some());
    assert_eq!(attempt_rows[1], (2, Some(200), None));
    let attempts = attempts.lock().expect("lock unsupported-model attempts");
    assert_eq!(attempts.get("Bearer upstream-primary").copied(), Some(1));
    assert_eq!(attempts.get("Bearer upstream-secondary").copied(), Some(1));
    drop(attempts);
    let primary_tags = sqlx::query_scalar::<_, String>("SELECT tag.system_key FROM pool_tags tag JOIN pool_upstream_account_tags link ON link.tag_id = tag.id WHERE link.account_id = ?1 AND tag.system_key IS NOT NULL ORDER BY tag.system_key ASC").bind(primary_id).fetch_all(&state.pool).await.expect("load primary account system tags");
    assert!(
        !primary_tags
            .iter()
            .any(|tag| tag == "unsupported_model:gpt-5.5")
    );
    let primary_model_route = load_model_routing_states(&state.pool, primary_id)
        .await
        .expect("load primary model route")
        .into_iter()
        .find(|route| route.model == "gpt-5.5")
        .expect("primary model route should be learned dynamically");
    assert_eq!(primary_model_route.failure_count, 1);
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, "sticky-unsupported-model-failover").await,
        Some(secondary_id)
    );
}

#[tokio::test]
async fn capture_target_pool_route_timeout_switches_to_alternate_upstream_route() {
    #[derive(Debug, sqlx::FromRow)]
    struct AttemptRouteRow {
        upstream_account_id: Option<i64>,
        upstream_route_key: Option<String>,
        attempt_index: i64,
        distinct_account_index: i64,
        same_account_retry_index: i64,
        status: String,
        failure_kind: Option<String>,
    }

    #[derive(Debug, sqlx::FromRow)]
    struct PersistedPayloadRow {
        payload: Option<String>,
    }

    let (slow_upstream_base, slow_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(750)).await;
    let (fast_upstream_base, _attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-fast", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(250);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let slow_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Slow Route",
        "route-slow",
        None,
        None,
        Some(slow_upstream_base.as_str()),
    )
    .await;
    let fast_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route",
        "route-fast",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;

    let sticky_key = "sticky-timeout-switch-001";
    let sticky_seen_at = format_test_recent_active_timestamp(Utc::now());
    upsert_test_sticky_route_at(&state.pool, sticky_key, slow_id, &sticky_seen_at).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            format!(r#"{{"model":"gpt-5","input":"hello","stickyKey":"{sticky_key}"}}"#)
                .into_bytes(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read timeout-switch success body");

    assert_timeout_route_switched(
        &state.pool,
        slow_id,
        fast_id,
        &slow_upstream_base,
        &fast_upstream_base,
    )
    .await;

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
}

async fn assert_timeout_route_switched(
    pool: &SqlitePool,
    slow_id: i64,
    fast_id: i64,
    slow_base: &str,
    fast_base: &str,
) {
    wait_for_codex_invocations(pool, 1).await;
    wait_for_pool_upstream_request_attempts(pool, 2).await;
    let rows = sqlx::query_as::<_, (Option<i64>, Option<String>, i64, i64, i64, String, Option<String>)>(
        "SELECT upstream_account_id, upstream_route_key, attempt_index, distinct_account_index, same_account_retry_index, status, failure_kind FROM pool_upstream_request_attempts ORDER BY attempt_index ASC",
    )
    .fetch_all(pool)
    .await
    .expect("load timeout-switch attempt rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, Some(slow_id));
    assert_eq!((rows[0].2, rows[0].3, rows[0].4), (1, 1, 1));
    assert_eq!(
        rows[0].5,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    assert_eq!(
        rows[0].6.as_deref(),
        Some(PROXY_FAILURE_UPSTREAM_STREAM_ERROR)
    );
    assert_eq!(rows[1].0, Some(fast_id));
    assert_eq!((rows[1].2, rows[1].3, rows[1].4), (2, 2, 1));
    assert_eq!(rows[1].5, POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS);
    assert_eq!(
        rows[0].1.as_deref(),
        Some(
            canonical_pool_upstream_route_key(
                &Url::parse(slow_base).expect("valid slow route url")
            )
            .as_str()
        )
    );
    assert_eq!(
        rows[1].1.as_deref(),
        Some(
            canonical_pool_upstream_route_key(
                &Url::parse(fast_base).expect("valid fast route url")
            )
            .as_str()
        )
    );
    let payload: Value = sqlx::query_scalar::<_, Option<String>>(
        "SELECT payload FROM codex_invocations ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("load timeout-switch invocation payload")
    .and_then(|payload| serde_json::from_str(&payload).ok())
    .expect("decode timeout-switch payload");
    assert_eq!(payload["poolAttemptCount"].as_i64(), Some(2));
    assert_eq!(payload["poolDistinctAccountCount"].as_i64(), Some(2));
    assert!(payload["poolAttemptTerminalReason"].is_null());
}

#[tokio::test]
async fn capture_target_pool_route_timeout_does_not_cut_out_from_no_cut_out_sticky_owner() {
    let (slow_upstream_base, slow_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(750)).await;
    let (fast_upstream_base, attempts, fast_upstream_handle) =
        spawn_pool_retry_upstream(&[("Bearer route-fast-no-cut-out", 0)]).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(250);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    let slow_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Slow No Cut Out Route",
        "route-slow-no-cut-out",
        None,
        None,
        Some(slow_upstream_base.as_str()),
    )
    .await;
    let fast_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Fast Route Behind No Cut Out",
        "route-fast-no-cut-out",
        None,
        None,
        Some(fast_upstream_base.as_str()),
    )
    .await;
    sqlx::query("UPDATE pool_upstream_accounts SET policy_allow_cut_out = 0 WHERE id = ?1")
        .bind(slow_id)
        .execute(&state.pool)
        .await
        .expect("set source no cut-out policy");

    let sticky_key = "sticky-timeout-no-cut-out-owner";
    let sticky_seen_at = format_test_recent_active_timestamp(Utc::now());
    upsert_test_sticky_route_at(&state.pool, sticky_key, slow_id, &sticky_seen_at).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            format!(r#"{{"model":"gpt-5","input":"hello","stickyKey":"{sticky_key}"}}"#)
                .into_bytes(),
        ),
    )
    .await;
    assert_no_cutout_timeout(&state, &attempts, response, slow_id, fast_id, sticky_key).await;

    slow_upstream_handle.abort();
    fast_upstream_handle.abort();
}

#[derive(Debug, sqlx::FromRow)]
struct NoCutoutAttemptRow {
    upstream_account_id: Option<i64>,
    attempt_index: i64,
    distinct_account_index: i64,
    same_account_retry_index: i64,
    status: String,
}

async fn assert_no_cutout_timeout(
    state: &Arc<AppState>,
    attempts: &Arc<StdMutex<HashMap<String, usize>>>,
    response: axum::response::Response,
    slow_id: i64,
    fast_id: i64,
    sticky_key: &str,
) {
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read no-cut-out timeout failure body");
    let response_payload: Value =
        serde_json::from_slice(&body).expect("decode no-cut-out timeout failure body");
    assert!(
        response_payload["error"]
            .as_str()
            .is_some_and(|message| message.contains("upstream")),
        "unexpected no-cut-out timeout payload: {response_payload}"
    );
    wait_for_codex_invocations(&state.pool, 1).await;
    wait_for_pool_attempt_row_count(&state.pool, 1).await;
    let attempt_rows = sqlx::query_as::<_, NoCutoutAttemptRow>("SELECT upstream_account_id, attempt_index, distinct_account_index, same_account_retry_index, status FROM pool_upstream_request_attempts ORDER BY attempt_index ASC").fetch_all(&state.pool).await.expect("load no-cut-out timeout attempt rows");
    assert_eq!(attempt_rows.len(), 1);
    assert_eq!(attempt_rows[0].upstream_account_id, Some(slow_id));
    assert_eq!(attempt_rows[0].attempt_index, 1);
    assert_eq!(attempt_rows[0].distinct_account_index, 1);
    assert_eq!(attempt_rows[0].same_account_retry_index, 1);
    assert_eq!(
        attempt_rows[0].status,
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    );
    let attempts = attempts.lock().expect("lock fast attempts");
    assert_eq!(attempts.get("Bearer route-fast-no-cut-out").copied(), None);
    drop(attempts);
    assert_eq!(
        load_test_sticky_route_account_id(&state.pool, sticky_key).await,
        Some(slow_id)
    );
    assert_ne!(
        load_test_sticky_route_account_id(&state.pool, sticky_key).await,
        Some(fast_id)
    );
}

#[test]
fn canonical_pool_upstream_route_key_collapses_trailing_slashes() {
    let without_trailing_slash =
        Url::parse("https://route.example/base?foo=bar").expect("valid route url");
    let with_trailing_slash =
        Url::parse("https://route.example/base/?baz=qux#frag").expect("valid route url");

    assert_eq!(
        canonical_pool_upstream_route_key(&without_trailing_slash),
        canonical_pool_upstream_route_key(&with_trailing_slash),
    );
    assert_eq!(
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example/?foo=bar").expect("valid root route url")
        ),
        "https://route.example/",
    );
    assert_eq!(
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example:443/base").expect("valid default https port route")
        ),
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example/base").expect("valid https route")
        ),
    );
    assert_eq!(
        canonical_pool_upstream_route_key(
            &Url::parse("http://route.example:80/base").expect("valid default http port route")
        ),
        canonical_pool_upstream_route_key(
            &Url::parse("http://route.example/base").expect("valid http route")
        ),
    );
    assert_ne!(
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example:8443/base")
                .expect("valid non-default https port route")
        ),
        canonical_pool_upstream_route_key(
            &Url::parse("https://route.example/base").expect("valid https route")
        ),
    );
}

#[test]
fn pool_failure_is_timeout_shaped_ignores_upstream_5xx_text_timeouts() {
    assert!(!pool_failure_is_timeout_shaped(
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "pool upstream responded with 500: operation timed out after 30s"
    ));
    assert!(pool_failure_is_timeout_shaped(
        PROXY_FAILURE_UPSTREAM_HANDSHAKE_TIMEOUT,
        "upstream handshake timed out after 60000ms"
    ));
    assert!(pool_failure_is_timeout_shaped(
        PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
        "request timed out after 120000ms while waiting for first upstream chunk"
    ));
}

#[tokio::test]
async fn capture_target_pool_route_timeout_returns_no_alternate_when_only_same_route_remains() {
    let (shared_upstream_base, shared_upstream_handle) =
        spawn_pool_delayed_first_chunk_upstream(Duration::from_millis(250)).await;
    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse("https://api.openai.com/").expect("valid upstream base url");
    config.pool_upstream_responses_attempt_timeout = Duration::from_millis(120);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-live-key").await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Shared Route A",
        "route-shared-a",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    insert_test_pool_api_key_account_with_options(
        &state,
        "Shared Route B",
        "route-shared-b",
        None,
        None,
        Some(shared_upstream_base.as_str()),
    )
    .await;
    let exhausted_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Exhausted Other Route",
        "route-exhausted",
        None,
        None,
        Some("https://exhausted.example.com/backend-api/codex"),
    )
    .await;
    insert_test_pool_limit_sample(&state, exhausted_id, Some(100.0), Some(0.0)).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer pool-live-key"),
        )]),
        Body::from(
            r#"{"model":"gpt-5","input":"hello","stickyKey":"sticky-timeout-no-alt-001"}"#
                .as_bytes()
                .to_vec(),
        ),
    )
    .await;
    assert_no_alternate_timeout(&state, response).await;

    shared_upstream_handle.abort();
}
