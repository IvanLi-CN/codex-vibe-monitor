pub(crate) async fn repair_active_account_activity_v2_coverage_best_effort(
    pool: &Pool<Sqlite>,
    hourly_rollup_sync_lock: &Mutex<()>,
    reason: &'static str,
) -> ActiveAccountActivityV2RepairResult {
    let gate = crate::db_pressure::global_db_pressure_gate();
    let _guard = hourly_rollup_sync_lock.lock().await;
    let _permit = match gate.try_begin_background("account_activity_v2_priority_repair") {
        Ok(permit) => permit,
        Err(deny_reason) => {
            debug!(
                reason,
                deny_reason = %deny_reason,
                wake_reason = "active_window_coverage_check",
                "active Dashboard coverage repair deferred by database pressure gate"
            );
            return ActiveAccountActivityV2RepairResult::Deferred;
        }
    };
    let _write_permit =
        match crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator()
            .try_acquire(crate::proxy_sqlite_write_coordinator::ProxySqliteWriteClass::P2Derived)
        {
            Some(permit) => permit,
            None => {
                debug!(
                    reason,
                    defer_reason = "coordinator_priority",
                    wake_reason = "active_window_coverage_check",
                    "active Dashboard coverage repair deferred before SQLite access"
                );
                return ActiveAccountActivityV2RepairResult::Deferred;
            }
        };
    let coordinator = crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator();
    let repair_result = tokio::select! {
        _ = coordinator.wait_for_p2_preemption() => {
            debug!(reason, wake_reason = "active_window_coverage_check", "active Dashboard coverage repair yielded to higher-priority SQLite writes");
            return ActiveAccountActivityV2RepairResult::Deferred;
        }
        result = repair_active_account_activity_v2_coverage(pool) => result,
    };
    match repair_result {
        Ok(outcome) => ActiveAccountActivityV2RepairResult::Repaired(outcome),
        Err(err) => {
            gate.record_error("account_activity_v2_priority_repair", &err);
            warn!(
                error = %err,
                reason,
                wake_reason = "active_window_coverage_check",
                "active Dashboard coverage repair failed"
            );
            ActiveAccountActivityV2RepairResult::Failed
        }
    }
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

pub(crate) async fn sweep_orphan_proxy_raw_files(
    pool: &Pool<Sqlite>,
    config: &AppConfig,
    raw_path_fallback_root: Option<&Path>,
    dry_run: bool,
) -> Result<usize> {
    let raw_dir = config.resolved_proxy_raw_dir();
    if !raw_dir.exists() {
        return Ok(0);
    }

    let referenced = sqlx::query_scalar::<_, String>(
        r#"
        SELECT path
        FROM (
            SELECT request_raw_path AS path
            FROM codex_invocations
            WHERE request_raw_path IS NOT NULL
            UNION
            SELECT response_raw_path AS path
            FROM codex_invocations
            WHERE response_raw_path IS NOT NULL
            UNION
            SELECT response_raw_path AS path
            FROM pool_upstream_request_attempts
            WHERE response_raw_path IS NOT NULL
        )
        WHERE path IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut referenced_paths = HashSet::new();
    for path in referenced {
        for candidate in resolved_raw_path_candidates(&path, raw_path_fallback_root) {
            referenced_paths.insert(candidate);
        }
    }

    let min_file_age = Duration::from_secs(DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS);
    let mut removed = 0usize;
    for entry in fs::read_dir(&raw_dir)
        .with_context(|| format!("failed to read raw payload directory {}", raw_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() {
            continue;
        }
        let age = match entry.metadata().and_then(|metadata| metadata.modified()) {
            Ok(modified) => modified.elapsed().unwrap_or_default(),
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to inspect orphan raw payload file age");
                continue;
            }
        };
        if age < min_file_age {
            continue;
        }
        let normalized = normalize_path_for_compare(&path);
        if referenced_paths.contains(&normalized) {
            continue;
        }
        if dry_run {
            removed += 1;
            continue;
        }
        match fs::remove_file(&path) {
            Ok(_) => removed += 1,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                warn!(path = %path.display(), error = %err, "failed to remove orphan raw payload file");
            }
        }
    }

    Ok(removed)
}

pub(crate) fn build_health_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/health", get(health_check))
        .route("/api/version", get(get_versions))
}

pub(crate) fn build_settings_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/settings", get(get_settings))
        .route(
            "/api/settings/external-api-keys",
            get(list_external_api_keys).post(create_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/:id/rotate",
            post(rotate_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/:id/disable",
            post(disable_external_api_key),
        )
        .route(
            "/api/settings/proxy-models",
            any(removed_proxy_model_settings_endpoint),
        )
        .route("/api/settings/proxy", put(put_proxy_settings))
        .route(
            "/api/settings/forward-proxy",
            put(put_forward_proxy_settings),
        )
        .route(
            "/api/settings/forward-proxy/validate",
            post(post_forward_proxy_candidate_validation),
        )
        .route(
            "/api/settings/forward-proxy/refresh-subscriptions",
            post(post_forward_proxy_refresh_subscriptions),
        )
        .route(
            "/api/settings/forward-proxy/nodes/:proxy_key/test-stream",
            get(stream_forward_proxy_node_latency_test),
        )
        .route(
            "/api/settings/forward-proxy/nodes/test-stream",
            get(stream_forward_proxy_nodes_latency_test),
        )
        .route("/api/settings/pricing", put(put_pricing_settings))
}

pub(crate) fn build_invocation_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/invocations", get(list_invocations))
        .route("/api/invocations/locate", get(locate_invocation))
        .route(
            "/api/invocations/:invoke_id/pool-attempts",
            get(fetch_invocation_pool_attempts),
        )
        .route(
            "/api/invocations/:id/detail",
            get(fetch_invocation_record_detail),
        )
        .route(
            "/api/invocations/:id/workflow-detail",
            get(fetch_invocation_workflow_detail),
        )
        .route(
            "/api/invocations/:id/response-body",
            get(fetch_invocation_response_body),
        )
        .route(
            "/api/invocations/:id/attempts/:attempt_public_id/response-body",
            get(fetch_invocation_attempt_response_body),
        )
        .route(
            "/api/invocations/:id/request-body",
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
    router
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
            "/api/stats/prompt-cache-conversation-binding-events/*encodedPromptCacheKey",
            get(list_prompt_cache_conversation_operation_events),
        )
        .route(
            "/api/stats/prompt-cache-conversation-bindings/reset-affinity/*encodedPromptCacheKey",
            post(post_prompt_cache_conversation_affinity_reset),
        )
        .route(
            "/api/stats/prompt-cache-conversation-bindings/*encodedPromptCacheKey",
            get(get_prompt_cache_conversation_binding)
                .patch(patch_prompt_cache_conversation_binding),
        )
        .route("/api/quota/latest", get(latest_quota_snapshot))
}

pub(crate) fn build_system_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/system/status", get(fetch_system_status))
        .route("/api/system/tasks", get(list_system_task_runs))
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
            "/api/pool/upstream-accounts/:account_id/call-attempts/locate",
            get(locate_upstream_account_attempt),
        )
        .route(
            "/api/pool/upstream-accounts/:account_id/call-attempts",
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
            "/api/pool/upstream-accounts/bulk-sync-jobs/:jobId/events",
            get(stream_bulk_upstream_account_sync_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/bulk-sync-jobs/:jobId",
            get(get_bulk_upstream_account_sync_job).delete(cancel_bulk_upstream_account_sync_job),
        )
        .route(
            "/api/pool/upstream-account-groups/*groupName",
            put(update_upstream_account_group).delete(delete_upstream_account_group),
        )
        .route(
            "/api/pool/upstream-accounts/:id/sticky-keys",
            get(get_upstream_account_sticky_keys),
        )
        .route(
            "/api/pool/upstream-accounts/:id/model-routing",
            get(get_upstream_account_model_routing),
        )
        .route(
            "/api/pool/upstream-accounts/:id/model-routing-events",
            get(list_upstream_account_model_routing_events),
        )
        .route(
            "/api/pool/upstream-accounts/:id/model-routing/reset",
            post(reset_upstream_account_model_routing),
        )
        .route(
            "/api/pool/upstream-accounts/:id/model-mappings",
            put(update_upstream_account_model_mappings),
        )
        .route(
            "/api/pool/upstream-accounts/:id",
            get(get_upstream_account)
                .patch(update_upstream_account)
                .delete(delete_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/:id/sync",
            post(sync_upstream_account),
        )
        .route(
            "/api/pool/upstream-accounts/:id/models/refresh",
            post(refresh_upstream_account_models),
        )
        .route(
            "/api/pool/upstream-accounts/:id/oauth/relogin",
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
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/:jobId/events",
            get(stream_imported_oauth_validation_job_events),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/imports/validation-jobs/:jobId",
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
            "/api/pool/upstream-accounts/oauth/mailbox-sessions/:sessionId",
            delete(delete_oauth_mailbox_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId",
            get(get_oauth_login_session).patch(update_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId/complete",
            post(complete_oauth_login_session),
        )
        .route(
            "/api/pool/upstream-accounts/oauth/login-sessions/:loginId/confirm-identity-overwrite",
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
            "/api/external/v1/upstream-accounts/oauth/:sourceAccountId",
            put(external_upsert_oauth_upstream_account_route)
                .patch(external_patch_oauth_upstream_account_route),
        )
        .route(
            "/api/external/v1/upstream-accounts/oauth/:sourceAccountId/relogin",
            post(external_relogin_oauth_upstream_account_route),
        )
}

pub(crate) fn build_proxy_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router.route("/v1/*path", any(proxy_openai_v1_with_connect_info))
}

pub(crate) fn build_app_router(state: Arc<AppState>) -> Router {
    build_proxy_routes(build_event_routes(build_external_routes(
        build_pool_routes(build_system_routes(build_stats_routes(
            build_invocation_routes(build_settings_routes(build_health_routes(Router::new()))),
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
    let mut router = build_app_router(state.clone())
        .layer(TraceLayer::new_for_http())
        .layer(cors_layer);

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
                .fallback_service(spa_service);
        } else {
            warn!(
                path = %index_file.display(),
                "static index.html not found; SPA fallback disabled"
            );
        }
    }

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
