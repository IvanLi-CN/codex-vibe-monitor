pub(crate) async fn spawn_pool_large_metadata_prefixed_response_failed_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/responses",
            post(pool_large_metadata_prefixed_response_failed_retry_upstream),
        )
        .with_state(PoolResponseFailedRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
            failure_code: UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
            failure_message: "Our servers are currently overloaded. Please try again later.",
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind large metadata-prefixed response.failed retry upstream");
    let addr = listener
        .local_addr()
        .expect("large metadata-prefixed response.failed retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("large metadata-prefixed response.failed retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

pub(crate) async fn spawn_pool_compact_overloaded_retry_upstream(
    fail_before_success: &[(&str, usize)],
) -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let fail_before_success = Arc::new(
        fail_before_success
            .iter()
            .map(|(authorization, failures)| ((*authorization).to_string(), *failures))
            .collect::<HashMap<_, _>>(),
    );
    let app = Router::new()
        .route(
            "/v1/responses/compact",
            post(pool_compact_overloaded_retry_upstream),
        )
        .with_state(PoolResponseFailedRetryUpstreamState {
            attempts: attempts.clone(),
            fail_before_success,
            failure_code: UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED,
            failure_message: "Our servers are currently overloaded. Please try again later.",
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind compact overloaded retry upstream");
    let addr = listener
        .local_addr()
        .expect("compact overloaded retry upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("compact overloaded retry upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

pub(crate) async fn spawn_pool_late_response_failed_upstream() -> (
    String,
    Arc<StdMutex<HashMap<String, usize>>>,
    JoinHandle<()>,
) {
    let attempts = Arc::new(StdMutex::new(HashMap::new()));
    let app = Router::new()
        .route("/v1/responses", post(pool_late_response_failed_upstream))
        .with_state(PoolLateResponseFailedUpstreamState {
            attempts: attempts.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool late response.failed upstream");
    let addr = listener
        .local_addr()
        .expect("pool late response.failed upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool late response.failed upstream should run");
    });
    (format!("http://{addr}"), attempts, handle)
}

pub(crate) async fn spawn_pool_http_failure_upstream(
    status: StatusCode,
    error_code: Option<&str>,
    error_message: &str,
) -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/responses", post(pool_http_failure_upstream))
        .with_state(PoolHttpFailureUpstreamState {
            status,
            error_code: error_code.map(str::to_string),
            error_message: error_message.to_string(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind pool http failure upstream");
    let addr = listener
        .local_addr()
        .expect("pool http failure upstream addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("pool http failure upstream should run");
    });
    (format!("http://{addr}"), handle)
}

pub(crate) async fn load_test_sticky_route_account_id(
    pool: &SqlitePool,
    sticky_key: &str,
) -> Option<i64> {
    sqlx::query_scalar(
        r#"
        SELECT account_id
        FROM (
            SELECT account_id, updated_at, 0 AS is_model_route
            FROM pool_sticky_routes
            WHERE sticky_key = ?1
            UNION ALL
            SELECT account_id, updated_at, 1 AS is_model_route
            FROM pool_sticky_model_routes
            WHERE sticky_key = ?1
        )
        ORDER BY is_model_route DESC, updated_at DESC
        LIMIT 1
        "#,
    )
    .bind(sticky_key)
    .fetch_optional(pool)
    .await
    .expect("load test sticky route")
}

pub(crate) async fn wait_for_pool_attempt_row_count(pool: &SqlitePool, min_count: i64) {
    for _ in 0..20 {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pool_upstream_request_attempts")
            .fetch_one(pool)
            .await
            .expect("count pool attempt rows");
        if count >= min_count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) async fn wait_for_test_sticky_route_account_id(
    pool: &SqlitePool,
    sticky_key: &str,
) -> Option<i64> {
    // The successful proxy response returns before its background route write has
    // committed. Stateful SQLite tests run concurrently, so retain a bounded
    // observation window for that asynchronous persistence.
    for _ in 0..50 {
        if let Some(account_id) = load_test_sticky_route_account_id(pool, sticky_key).await {
            return Some(account_id);
        }
        sleep(Duration::from_millis(25)).await;
    }
    None
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_keeps_existing_sticky_binding_when_source_is_over_soft_limit()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let primary_id = insert_test_pool_api_key_account(&state, "Primary", "upstream-primary").await;
    let secondary_id =
        insert_test_pool_api_key_account(&state, "Secondary", "upstream-secondary").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, primary_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, secondary_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, primary_id, Some(90.0), Some(90.0)).await;
    insert_test_pool_limit_sample(&state, secondary_id, Some(5.0), Some(5.0)).await;
    for sticky_key in [
        "sticky-bound",
        "sticky-bound-extra-001",
        "sticky-bound-extra-002",
        "sticky-bound-extra-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, primary_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-bound"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, primary_id);
    assert_ne!(account.account_id, secondary_id);
}

#[tokio::test]
pub(crate) async fn resolver_skips_degraded_accounts_for_fresh_assignment() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let degraded_id =
        insert_test_pool_api_key_account(&state, "Degraded", "upstream-degraded").await;
    let healthy_id = insert_test_pool_api_key_account(&state, "Healthy", "upstream-healthy").await;
    set_test_account_degraded_route_state(
        &state.pool,
        degraded_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        "test degraded plain 429",
    )
    .await;

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, healthy_id);
    assert_ne!(account.account_id, degraded_id);
    assert_eq!(
        account.routing_source,
        PoolRoutingSelectionSource::FreshAssignment
    );
}

#[tokio::test]
pub(crate) async fn resolver_keeps_existing_sticky_owner_reusable_during_temporary_cooldown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let degraded_id =
        insert_test_pool_api_key_account(&state, "Degraded", "upstream-degraded").await;
    let healthy_id = insert_test_pool_api_key_account(&state, "Healthy", "upstream-healthy").await;
    let sticky_seen_at = format_utc_iso(Utc::now());
    upsert_test_sticky_route_at(&state.pool, "sticky-degraded", degraded_id, &sticky_seen_at).await;
    set_test_account_degraded_route_state(
        &state.pool,
        degraded_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test degraded 5xx",
    )
    .await;

    let sticky_account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-degraded"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve degraded sticky account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("sticky degraded account should resolve, got {other:?}"),
    };

    assert_eq!(sticky_account.account_id, degraded_id);
    assert_eq!(
        sticky_account.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );

    set_test_account_generic_route_cooldown(&state.pool, degraded_id, 120).await;

    let sticky_account_during_cooldown = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-degraded"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve after temporary cooldown")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => {
            panic!("sticky owner should remain reusable during temporary cooldown, got {other:?}")
        }
    };

    assert_eq!(sticky_account_during_cooldown.account_id, degraded_id);
    assert_eq!(
        sticky_account_during_cooldown.routing_source,
        PoolRoutingSelectionSource::StickyReuse
    );
    assert_ne!(sticky_account_during_cooldown.account_id, healthy_id);
}

#[tokio::test]
pub(crate) async fn resolver_returns_degraded_only_when_only_temporary_failure_accounts_remain() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let plain_429_id =
        insert_test_pool_api_key_account(&state, "Plain429", "upstream-plain-429").await;
    let upstream_5xx_id =
        insert_test_pool_api_key_account(&state, "Upstream5xx", "upstream-5xx").await;
    set_test_account_degraded_route_state(
        &state.pool,
        plain_429_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
        "test degraded plain 429",
    )
    .await;
    set_test_account_degraded_route_state(
        &state.pool,
        upstream_5xx_id,
        FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
        "test degraded 5xx",
    )
    .await;

    let resolution = resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve degraded-only pool");
    assert!(matches!(resolution, PoolAccountResolution::DegradedOnly));
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_prefers_candidates_within_soft_sticky_limit() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let overloaded_id =
        insert_test_pool_api_key_account(&state, "Overloaded", "upstream-overloaded").await;
    let available_id =
        insert_test_pool_api_key_account(&state, "Available", "upstream-available").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, overloaded_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, available_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, overloaded_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, available_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-overloaded-001",
        "sticky-overloaded-002",
        "sticky-overloaded-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, overloaded_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-prefer-available"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, available_id);
    assert_ne!(account.account_id, overloaded_id);
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_falls_back_to_over_soft_limit_bucket_when_needed()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let preferred_id =
        insert_test_pool_api_key_account(&state, "Preferred", "upstream-preferred").await;
    let fallback_id =
        insert_test_pool_api_key_account(&state, "Fallback", "upstream-fallback").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, preferred_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, fallback_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, preferred_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, fallback_id, Some(65.0), Some(65.0)).await;
    for sticky_key in [
        "sticky-preferred-001",
        "sticky-preferred-002",
        "sticky-preferred-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, preferred_id, &recent_seen_at).await;
    }
    for sticky_key in [
        "sticky-fallback-001",
        "sticky-fallback-002",
        "sticky-fallback-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, fallback_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-soft-fallback"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, preferred_id);
    assert_ne!(account.account_id, fallback_id);
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_falls_back_after_soft_bucket_candidate_rejects_cut_in()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let source_id = insert_test_pool_api_key_account(&state, "Source", "upstream-source").await;
    let guarded_id = insert_test_pool_api_key_account(&state, "Guarded", "upstream-guarded").await;
    let overloaded_id =
        insert_test_pool_api_key_account(&state, "Overloaded", "upstream-overloaded").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now_iso = format_utc_iso(Utc::now());

    upsert_test_sticky_route_at(&state.pool, "sticky-transfer", source_id, &recent_seen_at).await;
    set_test_account_status(&state.pool, source_id, "error").await;
    set_test_account_local_limits(&state.pool, guarded_id, Some(100.0), Some(100.0)).await;
    set_test_account_local_limits(&state.pool, overloaded_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, guarded_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, overloaded_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-overloaded-cut-in-001",
        "sticky-overloaded-cut-in-002",
        "sticky-overloaded-cut-in-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, overloaded_id, &recent_seen_at).await;
    }

    let disallow_cut_in_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 1, 0, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("no-cut-in")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-in tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(guarded_id)
    .bind(disallow_cut_in_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-in tag");

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-transfer"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, overloaded_id);
    assert_ne!(account.account_id, guarded_id);
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_blocks_timeout_failover_past_cut_out_rule() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let source_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Source",
        "upstream-source",
        None,
        None,
        Some("https://route-a.example.com/backend-api/"),
    )
    .await;
    let _alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Alternate",
        "upstream-alternate",
        None,
        None,
        Some("https://route-b.example.com/backend-api/"),
    )
    .await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now_iso = format_utc_iso(Utc::now());

    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-timeout-cut-out",
        source_id,
        &recent_seen_at,
    )
    .await;

    let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("no-cut-out")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-out tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(source_id)
    .bind(disallow_cut_out_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-out tag");

    let mut excluded_upstream_route_keys = HashSet::new();
    excluded_upstream_route_keys.insert(
        crate::upstream_accounts::canonical_pool_upstream_route_key(
            &Url::parse("https://route-a.example.com/backend-api/").expect("valid route a url"),
        ),
    );

    let blocked = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-timeout-cut-out"),
        &[],
        &excluded_upstream_route_keys,
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::AssignedBlocked(blocked) => blocked,
        other => panic!("pool account should block after timeout route exclusion, got {other:?}"),
    };

    assert_eq!(blocked.account.account_id, source_id);
    assert!(
        blocked.message.contains("routing policy forbids it"),
        "unexpected blocked message: {}",
        blocked.message
    );
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_blocks_timeout_failover_past_cut_out_rule_with_invalid_sticky_group()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let source_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Broken Sticky Source",
        "upstream-broken-source",
        None,
        None,
        Some("https://route-a.example.com/backend-api/"),
    )
    .await;
    let _alternate_id = insert_test_pool_api_key_account_with_options(
        &state,
        "Healthy Alternate",
        "upstream-healthy-alternate",
        None,
        None,
        Some("https://route-b.example.com/backend-api/"),
    )
    .await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());
    let now_iso = format_utc_iso(Utc::now());

    upsert_test_sticky_route_at(
        &state.pool,
        "sticky-timeout-invalid-group",
        source_id,
        &recent_seen_at,
    )
    .await;

    let disallow_cut_out_tag_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO pool_tags (
            name,
            allow_cut_out, allow_cut_in, created_at, updated_at
        ) VALUES (?1, 0, 1, ?2, ?2)
        RETURNING id
        "#,
    )
    .bind("invalid-group-no-cut-out")
    .bind(&now_iso)
    .fetch_one(&state.pool)
    .await
    .expect("insert no-cut-out tag");
    sqlx::query(
        r#"
        INSERT INTO pool_upstream_account_tags (
            account_id, tag_id, created_at, updated_at
        ) VALUES (?1, ?2, ?3, ?3)
        "#,
    )
    .bind(source_id)
    .bind(disallow_cut_out_tag_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await
    .expect("attach no-cut-out tag");
    sqlx::query("UPDATE pool_upstream_accounts SET group_name = ?2 WHERE id = ?1")
        .bind(source_id)
        .bind("broken-sticky-group")
        .execute(&state.pool)
        .await
        .expect("set broken sticky group");

    let mut excluded_upstream_route_keys = HashSet::new();
    excluded_upstream_route_keys.insert(
        crate::upstream_accounts::canonical_pool_upstream_route_key(
            &Url::parse("https://route-a.example.com/backend-api/").expect("valid route a url"),
        ),
    );

    let blocked = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-timeout-invalid-group"),
        &[],
        &excluded_upstream_route_keys,
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::AssignedBlocked(blocked) => blocked,
        other => {
            panic!(
                "pool account should block after excluding no-cut-out sticky route, got {other:?}"
            )
        }
    };

    assert_eq!(blocked.account.account_id, source_id);
    assert!(
        blocked.message.contains("routing policy forbids it"),
        "unexpected blocked message: {}",
        blocked.message
    );
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_soft_deprioritizes_accounts_with_only_remote_limit_signals()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let exempt_id = insert_test_pool_api_key_account(&state, "Exempt", "upstream-exempt").await;
    let limited_id = insert_test_pool_api_key_account(&state, "Limited", "upstream-limited").await;
    let recent_seen_at = format_test_recent_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, limited_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, exempt_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, limited_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-exempt-001",
        "sticky-exempt-002",
        "sticky-exempt-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, exempt_id, &recent_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-exempt-target"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, limited_id);
    assert_ne!(account.account_id, exempt_id);
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_does_not_soft_deprioritize_stale_sticky_routes()
 {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let exempt_id =
        insert_test_pool_api_key_account(&state, "Exempt", "upstream-exempt-stale").await;
    let limited_id =
        insert_test_pool_api_key_account(&state, "Limited", "upstream-limited-stale").await;
    let stale_seen_at = format_test_stale_active_timestamp(Utc::now());

    set_test_account_local_limits(&state.pool, limited_id, Some(100.0), Some(100.0)).await;
    insert_test_pool_limit_sample(&state, exempt_id, Some(5.0), Some(5.0)).await;
    insert_test_pool_limit_sample(&state, limited_id, Some(80.0), Some(80.0)).await;
    for sticky_key in [
        "sticky-stale-exempt-001",
        "sticky-stale-exempt-002",
        "sticky-stale-exempt-003",
    ] {
        upsert_test_sticky_route_at(&state.pool, sticky_key, exempt_id, &stale_seen_at).await;
    }

    let account = match resolve_pool_account_for_request(
        state.as_ref(),
        Some("sticky-stale-exempt-target"),
        &[],
        &HashSet::new(),
    )
    .await
    .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, exempt_id);
    assert_ne!(account.account_id, limited_id);
}

#[tokio::test]
pub(crate) async fn resolve_pool_account_for_request_prefers_reset_aware_pressure_over_raw_percent()
{
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let team_id = insert_test_pool_api_key_account(&state, "Team", "upstream-team").await;
    let free_id = insert_test_pool_api_key_account(&state, "Free", "upstream-free").await;
    let now = Utc::now();

    insert_test_pool_limit_sample_with_windows(
        &state,
        TestPoolLimitSample {
            account_id: team_id,
            plan_type: Some("team"),
            primary_used_percent: Some(70.0),
            primary_window_minutes: Some(300),
            primary_resets_at: Some(&format_utc_iso(now + ChronoDuration::minutes(5))),
            secondary_used_percent: Some(40.0),
            secondary_window_minutes: Some(7 * 24 * 60),
            secondary_resets_at: Some(&format_utc_iso(now + ChronoDuration::days(1))),
        },
    )
    .await;
    insert_test_pool_limit_sample_with_windows(
        &state,
        TestPoolLimitSample {
            account_id: free_id,
            plan_type: Some("free"),
            primary_used_percent: None,
            primary_window_minutes: None,
            primary_resets_at: None,
            secondary_used_percent: Some(30.0),
            secondary_window_minutes: Some(7 * 24 * 60),
            secondary_resets_at: Some(&format_utc_iso(now + ChronoDuration::days(6))),
        },
    )
    .await;

    let account = match resolve_pool_account_for_request(state.as_ref(), None, &[], &HashSet::new())
        .await
        .expect("resolve pool account")
    {
        PoolAccountResolution::Resolved(account) => account,
        other => panic!("pool account should resolve, got {other:?}"),
    };

    assert_eq!(account.account_id, team_id);
    assert_ne!(account.account_id, free_id);
}

use super::*;
