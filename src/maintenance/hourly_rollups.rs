use super::*;

#[path = "hourly_rollups/archive_backfill.rs"]
mod archive_backfill;
#[path = "hourly_rollups/archive_replay.rs"]
mod archive_replay;
#[path = "hourly_rollups/bootstrap.rs"]
mod bootstrap;
#[path = "hourly_rollups/live_repair.rs"]
mod live_repair;

pub(crate) use archive_backfill::*;
pub(crate) use archive_replay::*;
#[cfg(test)]
use bootstrap::runtime_startup_hourly_rollup_refresh_scope;
pub(crate) use bootstrap::*;
pub(crate) use live_repair::*;

#[cfg(test)]
mod hourly_rollup_budget_tests {
    use super::*;

    #[test]
    fn runtime_startup_bootstrap_leaves_active_coverage_to_the_dedicated_task() {
        assert_eq!(
            runtime_startup_hourly_rollup_refresh_scope(),
            HourlyRollupRefreshScope::SkipActiveAccountActivityV2CoverageRepair
        );
    }

    #[tokio::test]
    async fn read_surface_refresh_cancels_while_waiting_for_sync_lock() {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .expect("create lazy sqlite pool");
        let lock = Arc::new(Mutex::new(()));
        let _held_guard = lock.lock().await;
        let cancel = CancellationToken::new();
        let refresh_lock = Arc::clone(&lock);
        let refresh_cancel = cancel.clone();
        let refresh = tokio::spawn(async move {
            refresh_hourly_rollups_for_read_surfaces_best_effort(
                &pool,
                refresh_lock.as_ref(),
                &refresh_cancel,
                "cancellation regression test",
                HourlyRollupRefreshScope::Full,
            )
            .await
        });

        tokio::task::yield_now().await;
        cancel.cancel();
        assert!(refresh.await.expect("refresh task should finish"));
    }
}

pub(crate) async fn replay_forward_proxy_archives_into_hourly_rollups_tx(
    tx: &mut SqliteConnection,
) -> Result<HistoricalRollupArchiveReplaySummary> {
    replay_forward_proxy_archives_into_hourly_rollups_tx_with_limits(
        tx,
        Instant::now(),
        None,
        None,
        0,
    )
    .await
}

pub(crate) async fn delete_rows_by_ids(
    tx: &mut sqlx::SqliteConnection,
    table: &str,
    ids: &[i64],
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut query = QueryBuilder::<Sqlite>::new(format!("DELETE FROM {table} WHERE id IN ("));
    {
        let mut separated = query.separated(", ");
        for id in ids {
            separated.push_bind(id);
        }
    }
    query.push(")");
    query.build().execute(&mut *tx).await?;
    Ok(())
}

#[path = "hourly_rollup_archive_support.rs"]
mod hourly_rollup_archive_support;
pub(crate) use hourly_rollup_archive_support::*;

pub(crate) fn build_health_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/health", get(health_check))
        .route("/api/version", get(get_versions))
}

pub(crate) fn build_invocation_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/invocations", get(list_invocations))
        .route("/api/invocations/locate", get(locate_invocation))
        .route(
            "/api/invocations/{invoke_id}/pool-attempts",
            get(fetch_invocation_pool_attempts),
        )
        .route(
            "/api/invocations/{id}/detail",
            get(fetch_invocation_record_detail),
        )
        .route(
            "/api/invocations/{id}/workflow-detail",
            get(fetch_invocation_workflow_detail),
        )
        .route(
            "/api/invocations/{id}/response-body",
            get(fetch_invocation_response_body),
        )
        .route(
            "/api/invocations/{id}/attempts/{attempt_public_id}/response-body",
            get(fetch_invocation_attempt_response_body),
        )
        .route(
            "/api/invocations/{id}/request-body",
            get(fetch_invocation_request_body),
        )
        .route("/api/invocations/summary", get(fetch_invocation_summary))
        .route(
            "/api/invocations/suggestions",
            get(fetch_invocation_suggestions),
        )
        .route(
            "/api/invocations/new-count",
            get(fetch_invocation_new_records_count),
        )
}

pub(crate) fn build_stats_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    let router = router
        .route("/api/stats", get(fetch_stats))
        .route(
            "/api/stats/long-term/overview",
            get(fetch_long_term_overview),
        )
        .route("/api/stats/long-term/series", get(fetch_long_term_series))
        .route("/api/stats/summary", get(fetch_summary))
        .route(
            "/api/stats/dashboard-activity",
            get(fetch_dashboard_activity),
        )
        .route(
            "/api/stats/dashboard-activity/recent",
            get(fetch_dashboard_activity_recent),
        )
        .route(
            "/api/stats/dashboard-network-timeseries",
            get(fetch_dashboard_network_timeseries),
        )
        .route(
            "/api/stats/dashboard-network-recent",
            get(fetch_dashboard_network_recent),
        )
        .route(
            "/api/stats/upstream-account-activity",
            get(fetch_upstream_account_activity),
        )
        .route(
            "/api/stats/forward-proxy",
            get(fetch_forward_proxy_live_stats),
        )
        .route(
            "/api/stats/forward-proxy/timeseries",
            get(fetch_forward_proxy_timeseries),
        )
        .route("/api/stats/timeseries", get(fetch_timeseries))
        .route(
            "/api/stats/parallel-work",
            get(fetch_parallel_work_stats_cached),
        )
        .route("/api/stats/perf", get(fetch_perf_stats))
        .route("/api/stats/errors", get(fetch_error_distribution))
        .route("/api/stats/failures/summary", get(fetch_failure_summary))
        .route("/api/stats/errors/others", get(fetch_other_errors))
        .route(
            "/api/stats/prompt-cache-conversations",
            get(fetch_prompt_cache_conversations),
        )
        .route(
            "/api/stats/prompt-cache-conversation-bindings/bulk-actions",
            post(post_bulk_prompt_cache_conversation_bindings),
        )
        .route(
            "/api/stats/prompt-cache-conversation-binding-events/{*encodedPromptCacheKey}",
            get(list_prompt_cache_conversation_operation_events),
        )
        .route(
            "/api/stats/prompt-cache-conversation-bindings/reset-affinity/{*encodedPromptCacheKey}",
            post(post_prompt_cache_conversation_affinity_reset),
        )
        .route(
            "/api/stats/prompt-cache-conversation-bindings/{*encodedPromptCacheKey}",
            get(get_prompt_cache_conversation_binding)
                .patch(patch_prompt_cache_conversation_binding),
        )
        .route("/api/quota/latest", get(latest_quota_snapshot));
    register_invocation_timeline_routes(router)
}

pub(crate) fn build_system_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/system/status", get(fetch_system_status))
        .route("/api/system/storage", get(fetch_system_storage))
        .route("/api/system/tasks", get(list_system_task_runs))
        .route("/api/system/managed-tasks", get(list_managed_tasks))
        .route(
            "/api/system/managed-tasks/runtime",
            get(get_managed_task_runtime),
        )
        .route(
            "/api/system/managed-tasks/timeline",
            get(get_managed_task_timeline),
        )
        .route(
            "/api/system/managed-tasks/{task_key}/workload",
            get(get_managed_task_workload),
        )
        .route(
            "/api/system/managed-tasks/{task_key}",
            get(get_managed_task).patch(update_managed_task),
        )
        .route(
            "/api/system/managed-tasks/{task_key}/run",
            post(run_managed_task_now),
        )
        .route(
            "/api/system/prompt-cache/materialization",
            get(fetch_prompt_cache_materialization_status)
                .patch(update_prompt_cache_materialization_control),
        )
        .route("/api/system/observability", get(observability_capabilities))
        .route(
            "/api/system/observability/hotpath/{report}",
            get(hotpath_report),
        )
        .route(
            "/api/system/performance",
            axum::routing::any(observability::performance_retired),
        )
        .route(
            "/api/system/performance/health",
            axum::routing::any(observability::performance_retired),
        )
        .route(
            "/api/system/performance/browser",
            axum::routing::any(observability::performance_retired)
                .layer(DefaultBodyLimit::max(BROWSER_MAX_BYTES)),
        )
}

pub(crate) fn build_pool_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/pool/routing-settings",
            get(get_pool_routing_settings).put(update_pool_routing_settings),
        )
        .route("/api/pool/tags", get(list_tags))
        .route(
            "/api/pool/forward-proxy-binding-nodes",
            get(list_forward_proxy_binding_nodes),
        )
        .route(
            "/api/pool/upstream-accounts",
            get(list_upstream_accounts_from_uri).post(bulk_update_upstream_accounts),
        )
        .route(
            "/api/pool/upstream-account-events",
            get(list_upstream_account_action_events),
        )
        .route("/api/pool/model-routing-live", get(get_model_routing_live))
        .route(
            "/api/pool/upstream-accounts/{account_id}/call-attempts/locate",
            get(locate_upstream_account_attempt),
        )
        .route(
            "/api/pool/upstream-accounts/{account_id}/call-attempts",
            get(list_upstream_account_attempts),
        )
        .route(
            "/api/pool/upstream-accounts/window-usage",
            post(get_upstream_account_window_usage),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs",
            post(create_bulk_upstream_account_sync_job),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/{jobId}/events",
            get(stream_bulk_upstream_account_sync_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/{jobId}",
            get(get_bulk_upstream_account_sync_job).delete(cancel_bulk_upstream_account_sync_job),
        )
        .route(
            "/api/pool/upstream-account-groups/{*groupName}",
            put(update_upstream_account_group).delete(delete_upstream_account_group),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/sticky-keys",
            get(get_upstream_account_sticky_keys),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/model-routing",
            get(get_upstream_account_model_routing),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/model-routing-events",
            get(list_upstream_account_model_routing_events),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/model-routing/reset",
            post(reset_upstream_account_model_routing),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/model-mappings",
            put(update_upstream_account_model_mappings),
        )
        .route(
            "/api/pool/upstream-accounts/{id}",
            get(get_upstream_account)
                .patch(update_upstream_account)
                .delete(delete_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/sync",
            post(sync_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/models/refresh",
            post(refresh_upstream_account_models),
        )
        .route(
            "/api/pool/upstream-accounts/{id}/oauth/relogin",
            post(relogin_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/api-keys",
            post(create_api_key_account),
        )
        .route(
            "/api/pool/upstream-accounts/api-keys/migration/preflight",
            post(preflight_api_key_group_migration),
        )
        .route(
            "/api/pool/upstream-accounts/api-keys/migration/confirm",
            post(confirm_api_key_group_migration),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions",
            post(create_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validate",
            post(validate_imported_oauth_accounts)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs",
            post(create_imported_oauth_validation_job)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/{jobId}/events",
            get(stream_imported_oauth_validation_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/{jobId}",
            delete(cancel_imported_oauth_validation_job),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports",
            post(import_validated_oauth_accounts)
                .layer(DefaultBodyLimit::max(IMPORTED_OAUTH_ROUTE_MAX_BODY_BYTES)),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions",
            post(create_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/status",
            post(get_oauth_mailbox_session_status),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/{sessionId}",
            delete(delete_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/{loginId}",
            get(get_oauth_login_session).patch(update_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/{loginId}/complete",
            post(complete_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/{loginId}/confirm-identity-overwrite",
            post(confirm_oauth_login_session_identity_overwrite),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/callback",
            get(oauth_callback),
        )
}

pub(crate) fn build_event_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/events", get(sse_stream))
}

pub(crate) fn build_external_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route(
            "/api/external/v1/upstream-accounts/oauth/{sourceAccountId}",
            put(external_upsert_oauth_upstream_account_route)
                .patch(external_patch_oauth_upstream_account_route),
        )
        .route(
            "/api/external/v1/upstream-accounts/oauth/{sourceAccountId}/relogin",
            post(external_relogin_oauth_upstream_account_route),
        )
}

pub(crate) fn build_proxy_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/v1/{*path}", any(proxy_openai_v1_with_connect_info))
}

pub(crate) fn build_app_router(state: Arc<AppState>) -> Router {
    apply_startup_readiness_gate(
        build_app_router_without_browser(state.clone()).merge(browser_ingest_router(state.clone())),
        state,
    )
}

fn apply_startup_readiness_gate(router: Router, state: Arc<AppState>) -> Router {
    router.layer(axum::middleware::from_fn_with_state(
        state,
        startup_readiness_gate,
    ))
}

async fn startup_readiness_gate(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    if request.uri().path() != "/health" && !state.startup_ready.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, "starting").into_response();
    }
    next.run(request).await
}

fn build_app_router_without_browser(state: Arc<AppState>) -> Router {
    build_proxy_routes(build_event_routes(build_external_routes(
        build_pool_routes(build_system_routes(build_stats_routes(
            build_invocation_routes(crate::api::build_settings_routes(build_health_routes(
                Router::new(),
            ))),
        ))),
    )))
    .with_state(state)
}

pub(crate) const SOCIAL_PREVIEW_RELATIVE_ATTR: &str = "content=\"/social-preview.png\"";
pub(crate) const SOCIAL_PREVIEW_PATH: &str = "/social-preview.png";

pub(crate) fn inject_absolute_social_preview_urls(
    index_html: String,
    headers: &HeaderMap,
    configured_public_origin: Option<&str>,
) -> String {
    let Some(origin) = request_public_origin(headers, configured_public_origin) else {
        return index_html;
    };
    let absolute_attr = format!("content=\"{origin}{SOCIAL_PREVIEW_PATH}\"");
    index_html.replace(SOCIAL_PREVIEW_RELATIVE_ATTR, &absolute_attr)
}

const PWA_METADATA_CACHE_CONTROL: &str = "no-cache, max-age=0, must-revalidate";
const PWA_ICON_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

pub(crate) fn static_cache_control(path: &str) -> Option<&'static str> {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let filename = path.rsplit('/').next().unwrap_or(path);
    if path == "/"
        || matches!(
            filename,
            "index.html" | "site.webmanifest" | "sw.js" | "version.json"
        )
    {
        return Some(PWA_METADATA_CACHE_CONTROL);
    }

    let icon_specs = [
        ("favicon-", "svg"),
        ("icon-192-", "png"),
        ("icon-512-", "png"),
        ("maskable-192-", "png"),
        ("maskable-512-", "png"),
    ];
    let (prefix, expected_extension) = icon_specs
        .iter()
        .find(|(prefix, _)| filename.starts_with(*prefix))?;
    let (digest, extension) = filename[prefix.len()..].rsplit_once('.')?;
    if digest.len() == 12
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        && extension == *expected_extension
    {
        return Some(PWA_ICON_CACHE_CONTROL);
    }
    None
}

fn apply_static_cache_control<B>(path: &str, response: &mut axum::http::Response<B>) {
    let Some(value) = static_cache_control(path) else {
        return;
    };
    response.headers_mut().insert(
        HeaderName::from_static("cache-control"),
        HeaderValue::from_static(value),
    );
}

pub(crate) async fn render_spa_index_response(
    state: Arc<AppState>,
    headers: &HeaderMap,
) -> Response {
    let Some(static_dir) = state.config.static_dir.as_ref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let index_file = static_dir.join("index.html");
    let index_html = match tokio::fs::read_to_string(&index_file).await {
        Ok(contents) => contents,
        Err(err) => {
            error!(path = %index_file.display(), ?err, "failed to read static index.html");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let mut response = Html(inject_absolute_social_preview_urls(
        index_html,
        headers,
        state.config.public_origin.as_deref(),
    ))
    .into_response();
    apply_static_cache_control("/index.html", &mut response);
    response
}

pub(crate) async fn spawn_http_server(
    state: Arc<AppState>,
) -> Result<(SocketAddr, JoinHandle<()>)> {
    let cors_layer = build_cors_layer(&state.config);
    let mut router = build_app_router_without_browser(state.clone());

    // Optionally attach headers in the future; standard EventSource cannot read headers

    if let Some(static_dir) = state.config.static_dir.clone() {
        let index_file = static_dir.join("index.html");
        if index_file.exists() {
            let index_state = state.clone();
            let spa_index_service = service_fn(move |request: Request<Body>| {
                let state = index_state.clone();
                let headers = request.headers().clone();
                async move { Ok::<_, Infallible>(render_spa_index_response(state, &headers).await) }
            });
            let index_html_state = state.clone();
            let spa_index_html_service = service_fn(move |request: Request<Body>| {
                let state = index_html_state.clone();
                let headers = request.headers().clone();
                async move { Ok::<_, Infallible>(render_spa_index_response(state, &headers).await) }
            });
            let fallback_state = state.clone();
            let spa_fallback = service_fn(move |request: Request<Body>| {
                let state = fallback_state.clone();
                let headers = request.headers().clone();
                async move { Ok::<_, Infallible>(render_spa_index_response(state, &headers).await) }
            });
            let static_service = ServeDir::new(static_dir).not_found_service(spa_fallback);
            let spa_service = service_fn(move |request: Request<Body>| {
                let path = request.uri().path().to_owned();
                let static_service = static_service.clone();
                async move {
                    let mut response = static_service.oneshot(request).await?;
                    apply_static_cache_control(&path, &mut response);
                    Ok::<_, Infallible>(response)
                }
            });
            router = router
                .route_service("/", spa_index_service)
                .route_service("/index.html", spa_index_html_service)
                .route_service("/{*path}", spa_service);
        } else {
            warn!(
                path = %index_file.display(),
                "static index.html not found; SPA fallback disabled"
            );
        }
    }

    router = router.merge(build_public_blog_runtime_router(state.clone()));
    // Profile matched templates once. Unmatched SPA paths may contain arbitrary user data;
    // browser ingestion is excluded from both profiler and application request metrics.
    if state.observability.enabled {
        router = router.route_layer(hotpath::AxumLayer::new());
    }
    router = router.merge(browser_ingest_router(state.clone()));
    router = apply_startup_readiness_gate(router, state.clone());
    router = router
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
            tracing::info_span!("request", method = %request.method(), uri_path = %observability::http_request_trace_path(request), version = ?request.version())
        }))
        .layer(cors_layer)
        .layer(axum::middleware::from_fn(
            observability::retired_performance_preflight,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            observability_http_middleware,
        ));

    let listener = TcpListener::bind(&state.config.http_bind).await?;
    let addr = listener.local_addr()?;
    info!(%addr, "http server listening");

    let shutdown = state.shutdown.clone();
    let handle = tokio::spawn(async move {
        if let Err(err) = serve_router_with_graceful_shutdown(listener, router, async move {
            shutdown.cancelled().await
        })
        .await
        {
            error!(?err, "http server exited with error");
        }
    });

    Ok((addr, handle))
}

#[cfg(test)]
mod social_preview_tests {
    use super::*;
    use axum::http::{HeaderValue, header};

    #[test]
    fn inject_absolute_social_preview_urls_rewrites_both_meta_tags() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            HeaderValue::from_static("monitor.example.com"),
        );
        headers.insert(
            header::HeaderName::from_static("x-forwarded-proto"),
            HeaderValue::from_static("https"),
        );
        let html = r#"
            <meta property="og:image" content="/social-preview.png" />
            <meta name="twitter:image" content="/social-preview.png" />
        "#
        .to_string();

        let rewritten = inject_absolute_social_preview_urls(html, &headers, None);

        assert!(rewritten.contains(r#"content="https://monitor.example.com/social-preview.png""#));
        assert_eq!(
            rewritten
                .matches("https://monitor.example.com/social-preview.png")
                .count(),
            2
        );
    }

    #[test]
    fn inject_absolute_social_preview_urls_prefers_configured_public_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:8080"));
        headers.insert(
            header::HeaderName::from_static("x-forwarded-host"),
            HeaderValue::from_static("monitor.example.com"),
        );
        let html = r#"
            <meta property="og:image" content="/social-preview.png" />
            <meta name="twitter:image" content="/social-preview.png" />
        "#
        .to_string();

        let rewritten = inject_absolute_social_preview_urls(
            html,
            &headers,
            Some("https://preview.example.com"),
        );

        assert!(rewritten.contains(r#"content="https://preview.example.com/social-preview.png""#));
    }

    #[test]
    fn static_cache_control_revalidates_pwa_metadata() {
        for path in [
            "/",
            "/index.html",
            "/site.webmanifest",
            "/sw.js",
            "/version.json",
        ] {
            assert_eq!(
                static_cache_control(path),
                Some(PWA_METADATA_CACHE_CONTROL),
                "{path}"
            );
        }
    }

    #[test]
    fn static_cache_control_immutably_caches_content_hashed_install_icons() {
        for path in [
            "/favicon-0123456789ab.svg",
            "/icon-192-0123456789ab.png",
            "/icon-512-0123456789ab.png",
            "/maskable-192-0123456789ab.png",
            "/maskable-512-0123456789ab.png?cache=bust",
        ] {
            assert_eq!(
                static_cache_control(path),
                Some(PWA_ICON_CACHE_CONTROL),
                "{path}"
            );
        }
        assert_eq!(static_cache_control("/icon-192.png"), None);
        assert_eq!(static_cache_control("/icon-192-0123456789ab.jpg"), None);
    }

    #[test]
    fn apply_static_cache_control_sets_the_selected_response_header() {
        let mut response = axum::http::Response::new(());
        apply_static_cache_control("/site.webmanifest", &mut response);
        assert_eq!(
            response.headers().get("cache-control").unwrap(),
            PWA_METADATA_CACHE_CONTROL
        );

        let mut response = axum::http::Response::new(());
        apply_static_cache_control("/icon-192-0123456789ab.png", &mut response);
        assert_eq!(
            response.headers().get("cache-control").unwrap(),
            PWA_ICON_CACHE_CONTROL
        );
    }
}

pub(crate) fn spawn_shutdown_signal_listener(cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        shutdown_listener().await;
        cancel.cancel();
        info!("shutdown signal received; beginning graceful shutdown");
    })
}

pub(crate) async fn shutdown_listener() {
    // Wait for Ctrl+C or SIGTERM (unix)
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub(crate) fn codex_invocations_create_sql(table_name: &str) -> String {
    format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table_name} (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'xy',
            model TEXT,
            input_tokens INTEGER,
            output_tokens INTEGER,
            cache_input_tokens INTEGER,
            reported_cache_write_tokens INTEGER,
            reasoning_tokens INTEGER,
            total_tokens INTEGER,
            cost REAL,
            cost_input REAL,
            cost_cache_write REAL,
            cost_cache_read REAL,
            cost_output REAL,
            cost_reasoning REAL,
            status TEXT,
            error_message TEXT,
            failure_kind TEXT,
            failure_class TEXT,
            is_actionable INTEGER NOT NULL DEFAULT 0,
            payload TEXT,
            raw_response TEXT NOT NULL,
            cost_estimated INTEGER NOT NULL DEFAULT 0,
            price_version TEXT,
            request_raw_path TEXT,
            request_raw_codec TEXT NOT NULL DEFAULT 'identity',
            request_raw_size INTEGER,
            request_raw_truncated INTEGER NOT NULL DEFAULT 0,
            request_raw_truncated_reason TEXT,
            response_raw_path TEXT,
            response_raw_codec TEXT NOT NULL DEFAULT 'identity',
            response_raw_size INTEGER,
            response_raw_truncated INTEGER NOT NULL DEFAULT 0,
            response_raw_truncated_reason TEXT,
            timeline_json TEXT,
            detail_level TEXT NOT NULL DEFAULT 'full',
            detail_pruned_at TEXT,
            detail_prune_reason TEXT,
            t_total_ms REAL,
            t_req_read_ms REAL,
            t_req_parse_ms REAL,
            t_upstream_connect_ms REAL,
            t_upstream_ttfb_ms REAL,
            first_token_ms REAL,
            t_upstream_stream_ms REAL,
            t_resp_parse_ms REAL,
            t_persist_ms REAL,
            created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
        table_name = table_name,
    )
}

pub(crate) async fn load_sqlite_table_columns(
    pool: &Pool<Sqlite>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

pub(crate) async fn load_sqlite_table_columns_from_connection(
    conn: &mut SqliteConnection,
    schema_name: Option<&str>,
    table_name: &str,
) -> Result<HashSet<String>> {
    let pragma = schema_name.map_or_else(
        || format!("PRAGMA table_info('{table_name}')"),
        |schema_name| format!("PRAGMA {schema_name}.table_info('{table_name}')"),
    );
    let columns = sqlx::query(&pragma)
        .fetch_all(&mut *conn)
        .await
        .with_context(|| format!("failed to inspect {table_name} schema"))?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();
    Ok(columns)
}

pub(crate) async fn ensure_pool_upstream_request_attempts_archive_schema(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns = load_sqlite_table_columns_from_connection(
        conn,
        Some("archive_db"),
        "pool_upstream_request_attempts",
    )
    .await?;
    for (column, ty) in [
        ("occurred_epoch_ms", "INTEGER"),
        ("attempt_public_id", "TEXT"),
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("upstream_base_url_host", "TEXT"),
        ("upstream_request_compression_algorithm", "TEXT"),
        ("upstream_request_compression_mode", "TEXT"),
        ("upstream_request_logical_body_bytes", "INTEGER"),
        ("upstream_request_transmitted_body_bytes", "INTEGER"),
        ("upstream_request_header_bytes_approx", "INTEGER"),
        ("upstream_response_body_bytes", "INTEGER"),
        ("upstream_response_header_bytes_approx", "INTEGER"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
        ("request_model", "TEXT"),
        ("upstream_request_model", "TEXT"),
        ("model_mapping_pattern", "TEXT"),
        ("routing_source", "TEXT"),
        ("routing_selection_audit_json", "TEXT"),
        ("request_summary_json", "TEXT"),
        ("response_summary_json", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("response_content_encoding", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement = format!(
                "ALTER TABLE archive_db.pool_upstream_request_attempts ADD COLUMN {column} {ty}"
            );
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to add archive_db.pool_upstream_request_attempts column {column}"
                    )
                })?;
        }
    }
    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS archive_db.idx_pool_upstream_request_attempts_public_id
        ON pool_upstream_request_attempts (attempt_public_id)
        WHERE attempt_public_id IS NOT NULL
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to ensure archive_db.idx_pool_upstream_request_attempts_public_id")?;
    Ok(())
}

pub(crate) async fn ensure_pool_upstream_request_attempts_archive_schema_in_place(
    conn: &mut SqliteConnection,
) -> Result<()> {
    let archive_columns =
        load_sqlite_table_columns_from_connection(conn, None, "pool_upstream_request_attempts")
            .await?;
    for (column, ty) in [
        ("occurred_epoch_ms", "INTEGER"),
        ("attempt_public_id", "TEXT"),
        ("upstream_route_key", "TEXT"),
        ("phase", "TEXT"),
        ("downstream_http_status", "INTEGER"),
        ("downstream_error_message", "TEXT"),
        ("upstream_base_url_host", "TEXT"),
        ("upstream_request_compression_algorithm", "TEXT"),
        ("upstream_request_compression_mode", "TEXT"),
        ("upstream_request_logical_body_bytes", "INTEGER"),
        ("upstream_request_transmitted_body_bytes", "INTEGER"),
        ("upstream_request_header_bytes_approx", "INTEGER"),
        ("upstream_response_body_bytes", "INTEGER"),
        ("upstream_response_header_bytes_approx", "INTEGER"),
        ("compact_support_status", "TEXT"),
        ("compact_support_reason", "TEXT"),
        ("group_name_snapshot", "TEXT"),
        ("proxy_binding_key_snapshot", "TEXT"),
        ("request_model", "TEXT"),
        ("upstream_request_model", "TEXT"),
        ("model_mapping_pattern", "TEXT"),
        ("routing_selection_audit_json", "TEXT"),
        ("request_summary_json", "TEXT"),
        ("response_summary_json", "TEXT"),
        ("response_raw_path", "TEXT"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_size", "INTEGER"),
        ("response_raw_truncated", "INTEGER NOT NULL DEFAULT 0"),
        ("response_raw_truncated_reason", "TEXT"),
        ("response_content_encoding", "TEXT"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE pool_upstream_request_attempts ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!(
                        "failed to add in-place pool_upstream_request_attempts archive column {column}"
                    )
                })?;
        }
    }
    sqlx::query(
        r#"
        CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_upstream_request_attempts_public_id
        ON pool_upstream_request_attempts (attempt_public_id)
        WHERE attempt_public_id IS NOT NULL
        "#,
    )
    .execute(&mut *conn)
    .await
    .context("failed to ensure idx_pool_upstream_request_attempts_public_id")?;
    Ok(())
}

pub(crate) async fn ensure_codex_invocations_archive_schema(
    conn: &mut SqliteConnection,
) -> Result<()> {
    ensure_codex_invocations_archive_schema_in_schema(conn, "archive_db").await
}

pub(crate) async fn ensure_codex_invocations_archive_schema_direct(
    conn: &mut SqliteConnection,
) -> Result<()> {
    ensure_codex_invocations_archive_schema_in_schema(conn, "main").await
}

async fn ensure_codex_invocations_archive_schema_in_schema(
    conn: &mut SqliteConnection,
    schema: &str,
) -> Result<()> {
    let table_prefix = if schema == "main" { "" } else { "archive_db." };
    let archive_columns = load_sqlite_table_columns_from_connection(
        conn,
        (schema != "main").then_some(schema),
        "codex_invocations",
    )
    .await?;
    for (column, ty) in [
        ("request_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("response_raw_codec", "TEXT NOT NULL DEFAULT 'identity'"),
        ("timeline_json", "TEXT"),
        ("cost_input", "REAL"),
        ("cost_cache_write", "REAL"),
        ("cost_cache_read", "REAL"),
        ("cost_output", "REAL"),
        ("cost_reasoning", "REAL"),
        ("reported_cache_write_tokens", "INTEGER"),
        ("first_token_ms", "REAL"),
    ] {
        if !archive_columns.contains(column) {
            let statement =
                format!("ALTER TABLE {table_prefix}codex_invocations ADD COLUMN {column} {ty}");
            sqlx::query(&statement)
                .execute(&mut *conn)
                .await
                .with_context(|| {
                    format!("failed to add {table_prefix}codex_invocations column {column}")
                })?;
        }
    }
    sqlx::query(&format!(
        r#"
        UPDATE {table_prefix}codex_invocations
        SET request_raw_codec = CASE
                WHEN request_raw_path IS NOT NULL AND request_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(request_raw_codec), '') = ''
           OR (request_raw_codec = 'identity' AND request_raw_path LIKE '%.gz')
        "#,
    ))
    .execute(&mut *conn)
    .await
    .context("failed to backfill archive_db.codex_invocations request_raw_codec")?;
    sqlx::query(&format!(
        r#"
        UPDATE {table_prefix}codex_invocations
        SET response_raw_codec = CASE
                WHEN response_raw_path IS NOT NULL AND response_raw_path LIKE '%.gz' THEN 'gzip'
                ELSE 'identity'
            END
        WHERE COALESCE(TRIM(response_raw_codec), '') = ''
           OR (response_raw_codec = 'identity' AND response_raw_path LIKE '%.gz')
        "#,
    ))
    .execute(&mut *conn)
    .await
    .context("failed to backfill archive_db.codex_invocations response_raw_codec")?;
    Ok(())
}

pub(crate) async fn migrate_codex_invocations_drop_raw_expires_at(
    pool: &Pool<Sqlite>,
    existing: &HashSet<String>,
) -> Result<()> {
    const TEMP_TABLE: &str = "codex_invocations_drop_raw_expires_at_new";

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to clear stale codex_invocations migration temp table")?;
    let create_temp_sql = codex_invocations_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to create codex_invocations migration temp table")?;

    let temp_pragma_sql = format!("PRAGMA table_info('{TEMP_TABLE}')");
    let new_columns: Vec<String> = sqlx::query(&temp_pragma_sql)
        .fetch_all(tx.as_mut())
        .await
        .context("failed to inspect codex_invocations migration temp schema")?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();
    let copy_columns: Vec<String> = new_columns
        .into_iter()
        .filter(|column| existing.contains(column))
        .collect();
    if copy_columns.is_empty() {
        bail!("codex_invocations migration found no shared columns to copy");
    }

    let copy_columns_csv = copy_columns.join(", ");
    let copy_sql = format!(
        "INSERT INTO {TEMP_TABLE} ({copy_columns_csv}) SELECT {copy_columns_csv} FROM codex_invocations"
    );
    sqlx::query(&copy_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to copy codex_invocations rows into migration temp table")?;
    sqlx::query("DROP TABLE codex_invocations")
        .execute(tx.as_mut())
        .await
        .context("failed to drop legacy codex_invocations table during migration")?;
    let rename_sql = format!("ALTER TABLE {TEMP_TABLE} RENAME TO codex_invocations");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to swap migrated codex_invocations table into place")?;
    tx.commit().await?;
    Ok(())
}
