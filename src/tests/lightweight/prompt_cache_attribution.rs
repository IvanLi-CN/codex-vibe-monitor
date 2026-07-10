use super::*;
use serde_json::json;

static COMPACT_ATTRIBUTION_TEST_LOCK: Lazy<std::sync::Mutex<()>> =
    Lazy::new(|| std::sync::Mutex::new(()));

fn client_headers(session_id: &str, window_id: &str, traceparent: &str) -> HeaderMap {
    client_headers_with_installation(session_id, window_id, Some("installation-a"), traceparent)
}

fn client_headers_with_installation(
    session_id: &str,
    window_id: &str,
    installation_id: Option<&str>,
    traceparent: &str,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("session_id"),
        HeaderValue::from_str(session_id).expect("valid session_id header"),
    );
    headers.insert(
        HeaderName::from_static("originator"),
        HeaderValue::from_static("codex"),
    );
    headers.insert(
        HeaderName::from_static("x-codex-window-id"),
        HeaderValue::from_str(window_id).expect("valid window id header"),
    );
    if let Some(installation_id) = installation_id {
        headers.insert(
            HeaderName::from_static("x-codex-installation-id"),
            HeaderValue::from_str(installation_id).expect("valid installation id header"),
        );
    }
    headers.insert(
        HeaderName::from_static("traceparent"),
        HeaderValue::from_str(traceparent).expect("valid traceparent header"),
    );
    headers
}

#[test]
fn compact_prompt_cache_attribution_ignores_installation_id_mismatch() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let response_context =
        client_prompt_cache_attribution_context_from_headers(&client_headers_with_installation(
            "session-a",
            "window-a",
            None,
            "00-00000000000000000000000000000001-0000000000000001-01",
        ));
    let compact_context =
        client_prompt_cache_attribution_context_from_headers(&client_headers_with_installation(
            "session-a",
            "window-a",
            Some("installation-a"),
            "00-00000000000000000000000000000002-0000000000000002-01",
        ));

    assert!(response_context.fingerprint.is_some());
    assert_eq!(response_context.cache_key, compact_context.cache_key);
    assert!(
        !response_context
            .header_fingerprints
            .contains_key("x-codex-installation-id")
    );
    assert!(
        compact_context
            .header_fingerprints
            .contains_key("x-codex-installation-id")
    );

    remember_prompt_cache_attribution(&response_context, "prompt-cache-a", Some("sticky-a"), now);

    let attribution =
        lookup_recent_prompt_cache_attribution(&compact_context, now + Duration::from_secs(30))
            .expect("compact should reuse attribution when only installation id differs");
    assert_eq!(attribution.prompt_cache_key, "prompt-cache-a");
    assert_eq!(attribution.sticky_key.as_deref(), Some("sticky-a"));
}

#[test]
fn compact_prompt_cache_attribution_reuses_recent_client_fingerprint() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let response_context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-a",
        "window-a",
        "00-00000000000000000000000000000001-0000000000000001-01",
    ));
    let compact_context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-a",
        "window-a",
        "00-00000000000000000000000000000002-0000000000000002-01",
    ));

    assert!(response_context.fingerprint.is_some());
    assert_eq!(response_context.cache_key, compact_context.cache_key);
    assert_ne!(
        response_context.header_fingerprints.get("traceparent"),
        compact_context.header_fingerprints.get("traceparent")
    );

    remember_prompt_cache_attribution(&response_context, "prompt-cache-a", Some("sticky-a"), now);

    let attribution =
        lookup_recent_prompt_cache_attribution(&compact_context, now + Duration::from_secs(30))
            .expect("recent compact should reuse prompt-cache attribution");
    assert_eq!(attribution.prompt_cache_key, "prompt-cache-a");
    assert_eq!(attribution.sticky_key.as_deref(), Some("sticky-a"));
}

#[test]
fn compact_prompt_cache_attribution_isolates_different_client_fingerprints() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let response_context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-a",
        "window-a",
        "00-00000000000000000000000000000001-0000000000000001-01",
    ));
    let compact_context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-b",
        "window-a",
        "00-00000000000000000000000000000002-0000000000000002-01",
    ));

    remember_prompt_cache_attribution(&response_context, "prompt-cache-a", None, now);

    assert!(
        lookup_recent_prompt_cache_attribution(&compact_context, now + Duration::from_secs(30))
            .is_none(),
        "compact from a different client fingerprint must not inherit another conversation key"
    );
}

#[test]
fn compact_prompt_cache_attribution_expires_after_ttl() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-a",
        "window-a",
        "00-00000000000000000000000000000001-0000000000000001-01",
    ));
    remember_prompt_cache_attribution(&context, "prompt-cache-a", None, now);

    assert!(
        lookup_recent_prompt_cache_attribution(&context, now + Duration::from_secs(16 * 60))
            .is_none(),
        "attribution should expire before it can bridge unrelated later compact requests"
    );
}

#[test]
fn compact_prompt_cache_attribution_rejects_ambiguous_recent_keys() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-a",
        "window-a",
        "00-00000000000000000000000000000001-0000000000000001-01",
    ));

    remember_prompt_cache_attribution(&context, "prompt-cache-a", None, now);
    remember_prompt_cache_attribution(
        &context,
        "prompt-cache-b",
        None,
        now + Duration::from_secs(20),
    );

    assert!(
        lookup_recent_prompt_cache_attribution(&context, now + Duration::from_secs(30)).is_none(),
        "same client fingerprint with multiple recent keys must stay unattributed"
    );
}

#[test]
fn compact_prompt_cache_attribution_recovers_after_older_conflict_expires() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let context = client_prompt_cache_attribution_context_from_headers(&client_headers(
        "session-a",
        "window-a",
        "00-00000000000000000000000000000001-0000000000000001-01",
    ));

    remember_prompt_cache_attribution(&context, "prompt-cache-a", None, now);
    remember_prompt_cache_attribution(
        &context,
        "prompt-cache-b",
        Some("sticky-b"),
        now + Duration::from_secs(20),
    );

    assert!(
        lookup_recent_prompt_cache_attribution(&context, now + Duration::from_secs(30)).is_none(),
        "multiple recent keys for the same client fingerprint should stay unattributed"
    );

    let attribution =
        lookup_recent_prompt_cache_attribution(&context, now + Duration::from_secs(15 * 60 + 1))
            .expect("newer key should become attributable after the older conflict expires");
    assert_eq!(attribution.prompt_cache_key, "prompt-cache-b");
    assert_eq!(attribution.sticky_key.as_deref(), Some("sticky-b"));
}

#[test]
fn compact_prompt_cache_attribution_requires_unique_client_key() {
    let _guard = COMPACT_ATTRIBUTION_TEST_LOCK.lock().expect("test lock");
    clear_prompt_cache_attribution_for_tests();
    let now = Instant::now();
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("originator"),
        HeaderValue::from_static("codex"),
    );
    headers.insert(
        HeaderName::from_static("x-codex-installation-id"),
        HeaderValue::from_static("installation-a"),
    );
    headers.insert(
        HeaderName::from_static("traceparent"),
        HeaderValue::from_static("00-00000000000000000000000000000001-0000000000000001-01"),
    );
    let context = client_prompt_cache_attribution_context_from_headers(&headers);

    assert!(context.cache_key.is_none());
    assert!(context.fingerprint.is_none());
    assert!(context.header_fingerprints.contains_key("traceparent"));

    remember_prompt_cache_attribution(&context, "prompt-cache-a", None, now);
    assert!(
        lookup_recent_prompt_cache_attribution(&context, now + Duration::from_secs(30)).is_none()
    );
}

#[test]
fn compact_request_body_still_avoids_rewrite_and_usage_injection() {
    let request_body = br#"{"model":"gpt-5","stream":true,"service_tier":"default"}"#.to_vec();
    let (upstream_body, info, rewritten) = prepare_target_request_body(
        ProxyCaptureTarget::ResponsesCompact,
        request_body.clone(),
        true,
    );

    assert!(!rewritten);
    assert_eq!(upstream_body, request_body);
    assert_eq!(info.requested_service_tier.as_deref(), Some("default"));
    assert!(info.prompt_cache_key.is_none());

    let upstream_json: Value =
        serde_json::from_slice(&upstream_body).expect("compact request remains valid JSON");
    assert!(upstream_json.get("stream_options").is_none());
    assert!(!ProxyCaptureTarget::ResponsesCompact.allows_fast_mode_rewrite());
    assert!(!ProxyCaptureTarget::ResponsesCompact.should_auto_include_usage());
}

#[tokio::test]
async fn prompt_cache_recent_invocations_include_attributed_compact_preview() {
    let state = test_state_from_config(test_config(), true).await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, total_tokens, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("compact-attribution-response")
    .bind("2026-04-27 10:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        json!({
            "endpoint": "/v1/responses",
            "promptCacheKey": "prompt-cache-attributed",
            "stickyKey": "sticky-attributed"
        })
        .to_string(),
    )
    .bind(12_i64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert keyed response invocation");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, total_tokens, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("compact-attribution-compact")
    .bind("2026-04-27 10:01:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        json!({
            "endpoint": "/v1/responses/compact",
            "compactionRequestKind": "compact",
            "compactionResponseKind": "compact",
            "promptCacheKey": "prompt-cache-attributed",
            "stickyKey": "sticky-attributed",
            "promptCacheKeyAttributionSource": "client_fingerprint_recent"
        })
        .to_string(),
    )
    .bind(4_i64)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert attributed compact invocation");

    let rows = query_prompt_cache_conversation_recent_invocations(
        &state.pool,
        InvocationSourceScope::ProxyOnly,
        &["prompt-cache-attributed".to_string()],
        5,
        None,
    )
    .await
    .expect("query prompt-cache recent invocation previews");

    let compact = rows
        .iter()
        .find(|row| row.invoke_id == "compact-attribution-compact")
        .expect("attributed compact should appear in conversation previews");
    assert_eq!(compact.endpoint.as_deref(), Some("/v1/responses/compact"));
    assert_eq!(compact.compaction_request_kind.as_deref(), Some("compact"));
    assert_eq!(compact.compaction_response_kind.as_deref(), Some("compact"));
    assert_eq!(compact.prompt_cache_key, "prompt-cache-attributed");
}
