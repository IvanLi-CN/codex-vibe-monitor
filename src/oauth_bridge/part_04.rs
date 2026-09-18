#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, body::to_bytes, routing::post};
    use tokio::net::TcpListener;

    #[test]
    fn transform_models_payload_maps_codex_catalog_to_openai_shape() {
        let payload = br#"{"models":[{"slug":"gpt-5.4"},{"slug":"gpt-5.3-codex"}]}"#;
        let value = transform_models_payload(payload).expect("transform models payload");
        assert_eq!(value["object"], "list");
        assert_eq!(value["data"][0]["id"], "gpt-5.4");
        assert_eq!(value["data"][1]["id"], "gpt-5.3-codex");
    }

    #[test]
    fn extract_completed_response_from_sse_returns_completed_response() {
        let payload = b"event: response.created\n\ndata: {\"type\":\"response.created\"}\n\nevent: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_123\",\"status\":\"completed\"}}\n\n";
        let value =
            extract_completed_response_from_sse(payload).expect("extract completed response");
        assert_eq!(value["id"], "resp_123");
        assert_eq!(value["status"], "completed");
    }

    #[tokio::test]
    async fn oauth_codex_upstream_base_url_uses_test_override() {
        let _guard = TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK.lock().await;
        let override_url =
            Url::parse("http://127.0.0.1:43123/backend-api/codex").expect("valid override url");
        set_test_oauth_codex_upstream_base_url(override_url.clone()).await;
        assert_eq!(
            oauth_codex_upstream_base_url()
                .expect("oauth codex upstream base url")
                .as_str(),
            override_url.as_str()
        );
        reset_test_oauth_codex_upstream_base_url().await;
    }

    #[test]
    fn unsupported_oauth_route_returns_explicit_error() {
        assert!(!is_supported_oauth_passthrough_route(
            &Method::POST,
            "/v1/embeddings"
        ));
        assert!(is_supported_oauth_passthrough_route(
            &Method::POST,
            "/v1/responses/compact"
        ));
        assert!(is_supported_oauth_passthrough_route(
            &Method::POST,
            "/v1/chat/completions"
        ));
        assert!(is_supported_oauth_passthrough_route(
            &Method::POST,
            "/v1/alpha/search"
        ));
        assert!(!is_supported_oauth_passthrough_route(
            &Method::GET,
            "/v1/alpha/search"
        ));
        assert!(!is_supported_oauth_passthrough_route(
            &Method::PUT,
            "/v1/alpha/search"
        ));
    }

    #[tokio::test]
    async fn standalone_search_passthrough_supports_ordinary_and_counted_requests() {
        async fn search_upstream(request: axum::extract::Request) -> Response {
            let body = to_bytes(request.into_body(), usize::MAX)
                .await
                .expect("read search request body");
            (
                StatusCode::ACCEPTED,
                Json(json!({
                    "path": "/backend-api/codex/alpha/search",
                    "query": "cursor=next",
                    "body": String::from_utf8(body.to_vec()).expect("search body is utf8"),
                })),
            )
                .into_response()
        }

        let _guard = TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK.lock().await;
        let handle = start_standalone_search_oauth_upstream(search_upstream).await;

        let headers = HeaderMap::from_iter([(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]);
        let uri = "/v1/alpha/search?cursor=next"
            .parse()
            .expect("valid search uri");
        let request_body = Bytes::from_static(br#"{"model":"gpt-5.4","query":"hello"}"#);
        let ordinary = send_oauth_upstream_request(
            &Client::new(),
            OauthUpstreamRequestContext {
                method: Method::POST,
                original_uri: &uri,
                headers: &headers,
                handshake_timeout: Duration::from_secs(5),
                response_timeout: Duration::from_secs(5),
                account_id: Some(7),
                access_token: "oauth-search-ordinary",
                chatgpt_account_id: Some("org_search"),
                installation_seed: None,
                crypto_key: None,
            },
            OauthUpstreamRequestBody::Bytes(request_body.clone()),
        )
        .await;
        assert_eq!(ordinary.response.status(), StatusCode::ACCEPTED);
        let ordinary_body = to_bytes(ordinary.response.into_body(), usize::MAX)
            .await
            .expect("read ordinary search response");
        let ordinary_payload: Value =
            serde_json::from_slice(&ordinary_body).expect("decode ordinary search response");
        assert_eq!(ordinary_payload["path"], "/backend-api/codex/alpha/search");
        assert_eq!(ordinary_payload["query"], "cursor=next");
        assert_eq!(
            ordinary_payload["body"],
            "{\"model\":\"gpt-5.4\",\"query\":\"hello\"}"
        );

        let counted = send_counted_oauth_upstream_request(
            CountedOauthUpstreamRequestContext {
                request: OauthUpstreamRequestContext {
                    method: Method::POST,
                    original_uri: &uri,
                    headers: &headers,
                    handshake_timeout: Duration::from_secs(5),
                    response_timeout: Duration::from_secs(5),
                    account_id: Some(7),
                    access_token: "oauth-search-counted",
                    chatgpt_account_id: Some("org_search"),
                    installation_seed: None,
                    crypto_key: None,
                },
                forward_proxy_url: None,
                reporter: None,
            },
            CountedOauthUpstreamRequestBody::Bytes(request_body),
        )
        .await;
        assert_eq!(counted.response.status(), StatusCode::ACCEPTED);
        let counted_body = to_bytes(counted.response.into_body(), usize::MAX)
            .await
            .expect("read counted search response");
        let counted_payload: Value =
            serde_json::from_slice(&counted_body).expect("decode counted search response");
        assert_eq!(counted_payload["path"], "/backend-api/codex/alpha/search");
        assert_eq!(counted_payload["query"], "cursor=next");

        handle.abort();
        reset_test_oauth_codex_upstream_base_url().await;
    }

    async fn start_standalone_search_oauth_upstream(
        handler: impl axum::handler::Handler<(), ()> + Clone + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let app = Router::new().route("/backend-api/codex/alpha/search", post(handler));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind standalone search oauth upstream");
        let addr = listener
            .local_addr()
            .expect("standalone search oauth upstream addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("standalone search oauth upstream should run");
        });
        set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("http://{addr}/backend-api/codex"))
                .expect("valid standalone search oauth base url"),
        )
        .await;
        handle
    }

    #[test]
    fn attach_account_header_accepts_uuid_style_account_ids() {
        let request = attach_account_header(
            Client::new().get("https://example.com"),
            Some(" 02355c9d-fb23-4517-a96d-35e5f6758e9e "),
        )
        .build()
        .expect("build request");

        assert_eq!(
            request
                .headers()
                .get("ChatGPT-Account-Id")
                .and_then(|value| value.to_str().ok()),
            Some("02355c9d-fb23-4517-a96d-35e5f6758e9e")
        );
    }

    #[test]
    fn attach_account_header_skips_blank_account_ids() {
        let request = attach_account_header(Client::new().get("https://example.com"), Some("   "))
            .build()
            .expect("build request");

        assert!(request.headers().get("ChatGPT-Account-Id").is_none());
    }

    #[test]
    fn prepare_responses_request_body_preserves_previous_response_id() {
        let prepared = prepare_responses_request_body(
            br#"{"model":"gpt-5.4","stream":false,"max_output_tokens":256,"previous_response_id":"resp_prev_001"}"#,
            None,
            None,
        )
        .expect("rewrite responses request");

        assert!(!prepared.wants_stream);
        let payload: Value = serde_json::from_slice(&prepared.body).expect("decode rewritten body");
        assert_eq!(payload["previous_response_id"], "resp_prev_001");
        assert_eq!(payload["instructions"], "");
        assert_eq!(payload["store"], false);
        assert_eq!(payload["stream"], true);
        assert!(payload.get("max_output_tokens").is_none());
        assert_eq!(
            prepared.rewrite,
            OauthResponsesRewriteSummary {
                applied: true,
                added_instructions: true,
                added_store: true,
                forced_stream_true: true,
                removed_max_output_tokens: true,
                rewrote_installation_id: false,
                removed_installation_id: false,
            }
        );
    }

    #[test]
    fn derive_oauth_installation_id_is_stable_and_distinct_per_account() {
        let seed = [0x5a_u8; 32];
        let first = derive_oauth_installation_id(&seed, 42);
        let same_again = derive_oauth_installation_id(&seed, 42);
        let different_account = derive_oauth_installation_id(&seed, 43);

        assert_eq!(first, same_again);
        assert_ne!(first, different_account);
        assert_eq!(first.len(), 36);
        assert_eq!(first.chars().nth(8), Some('-'));
        assert_eq!(first.chars().nth(13), Some('-'));
        assert_eq!(first.chars().nth(18), Some('-'));
        assert_eq!(first.chars().nth(23), Some('-'));
        assert!(first.chars().all(|ch| ch == '-' || ch.is_ascii_hexdigit()));
        assert_eq!(first, first.to_ascii_lowercase());
    }

    #[test]
    fn prepare_responses_request_body_rewrites_installation_id_when_account_id_present() {
        let seed = [0x11_u8; 32];
        let expected_installation_id = derive_oauth_installation_id(&seed, 7);
        let prepared = prepare_responses_request_body(
            br#"{
                "model":"gpt-5.4",
                "stream":false,
                "max_output_tokens":256,
                "client_metadata":{
                    "x-codex-installation-id":"downstream-installation-id",
                    "other":"keep-me"
                }
            }"#,
            Some(7),
            Some(&seed),
        )
        .expect("rewrite responses request");

        let payload: Value = serde_json::from_slice(&prepared.body).expect("decode rewritten body");
        assert_eq!(
            payload["client_metadata"]["x-codex-installation-id"],
            Value::String(expected_installation_id)
        );
        assert_eq!(payload["client_metadata"]["other"], "keep-me");
        assert_eq!(
            prepared.rewrite,
            OauthResponsesRewriteSummary {
                applied: true,
                added_instructions: true,
                added_store: true,
                forced_stream_true: true,
                removed_max_output_tokens: true,
                rewrote_installation_id: true,
                removed_installation_id: false,
            }
        );
    }

    #[test]
    fn prepare_responses_request_body_strips_installation_id_without_account_id() {
        let seed = [0x22_u8; 32];
        let prepared = prepare_responses_request_body(
            br#"{
                "model":"gpt-5.4",
                "stream":true,
                "instructions":"",
                "store":false,
                "client_metadata":{
                    "x-codex-installation-id":"downstream-installation-id",
                    "other":"keep-me"
                }
            }"#,
            None,
            Some(&seed),
        )
        .expect("rewrite responses request");

        let payload: Value = serde_json::from_slice(&prepared.body).expect("decode rewritten body");
        assert!(
            payload["client_metadata"]
                .get("x-codex-installation-id")
                .is_none()
        );
        assert_eq!(payload["client_metadata"]["other"], "keep-me");
        assert_eq!(
            prepared.rewrite,
            OauthResponsesRewriteSummary {
                applied: true,
                added_instructions: false,
                added_store: false,
                forced_stream_true: false,
                removed_max_output_tokens: false,
                rewrote_installation_id: false,
                removed_installation_id: true,
            }
        );
    }

    #[tokio::test]
    async fn load_or_init_oauth_installation_seed_persists_single_value() {
        let db_url = format!(
            "sqlite:file:oauth-installation-seed-test-{}?mode=memory&cache=shared",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time should be after unix epoch")
                .as_nanos()
        );
        let pool = sqlx::SqlitePool::connect(&db_url)
            .await
            .expect("connect in-memory sqlite");
        crate::ensure_schema(&pool)
            .await
            .expect("schema should initialize");

        let first = load_or_init_oauth_installation_seed(&pool)
            .await
            .expect("load or init oauth installation seed");
        let second = load_or_init_oauth_installation_seed(&pool)
            .await
            .expect("reload oauth installation seed");
        let row_count =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM oauth_bridge_settings WHERE id = 1")
                .fetch_one(&pool)
                .await
                .expect("count oauth bridge settings rows");

        assert_eq!(first, second);
        assert_eq!(row_count, 1);
    }

    #[test]
    fn copy_forwardable_headers_keeps_prompt_cache_headers_but_strips_sticky_headers() {
        let client = Client::new();
        let crypto_key: [u8; 32] = Sha256::digest(b"oauth-debug-test-secret").into();
        let headers = HeaderMap::from_iter([
            (
                header::HeaderName::from_static("x-prompt-cache-key"),
                "prompt-cache-alpha".parse().expect("x prompt cache value"),
            ),
            (
                header::HeaderName::from_static("x-openai-prompt-cache-key"),
                "prompt-cache-beta"
                    .parse()
                    .expect("openai prompt cache value"),
            ),
            (
                header::HeaderName::from_static("x-client-trace-id"),
                "trace-123".parse().expect("client trace id"),
            ),
            (
                header::HeaderName::from_static("x-sticky-key"),
                "sticky-should-not-forward".parse().expect("sticky key"),
            ),
        ]);

        let (builder, summary) = copy_forwardable_headers(
            client.get("https://example.com"),
            &headers,
            &[],
            Some(&crypto_key),
        );
        let request = builder.build().expect("build request");

        assert_eq!(
            request
                .headers()
                .get("x-prompt-cache-key")
                .and_then(|value| value.to_str().ok()),
            Some("prompt-cache-alpha")
        );
        assert_eq!(
            request
                .headers()
                .get("x-openai-prompt-cache-key")
                .and_then(|value| value.to_str().ok()),
            Some("prompt-cache-beta")
        );
        assert_eq!(
            request
                .headers()
                .get("x-client-trace-id")
                .and_then(|value| value.to_str().ok()),
            Some("trace-123")
        );
        assert!(request.headers().get("x-sticky-key").is_none());
        assert!(summary.prompt_cache_header_forwarded);
        assert_eq!(
            summary.names,
            vec![
                "x-client-trace-id".to_string(),
                "x-openai-prompt-cache-key".to_string(),
                "x-prompt-cache-key".to_string()
            ]
        );
        assert_eq!(
            summary.fingerprints,
            Some(BTreeMap::new()),
            "non-allowlisted forwarded headers should not emit fingerprints"
        );
    }

    #[test]
    fn oauth_responses_request_overrides_security_headers_after_forwarding() {
        let client = Client::new();
        let headers = HeaderMap::from_iter([
            (
                header::AUTHORIZATION,
                "Bearer client-token".parse().expect("authorization"),
            ),
            (
                header::CONTENT_TYPE,
                "application/custom+json".parse().expect("content type"),
            ),
            (
                header::CONTENT_LENGTH,
                "999".parse().expect("content length"),
            ),
            (
                header::HeaderName::from_static("chatgpt-account-id"),
                "client-account".parse().expect("chatgpt account id"),
            ),
            (
                header::HeaderName::from_static("x-openai-prompt-cache-key"),
                "prompt-cache-gamma".parse().expect("prompt cache key"),
            ),
        ]);

        let builder = client.post("https://example.com");
        let (builder, _) = copy_forwardable_headers(
            builder,
            &headers,
            OAUTH_RESPONSES_EXCLUDED_HEADER_NAMES,
            None,
        );
        let request = attach_account_header(
            builder
                .bearer_auth("oauth-upstream-token")
                .header(header::CONTENT_TYPE, "application/json")
                .header("OpenAI-Beta", "responses=experimental"),
            Some("server-account"),
        )
        .build()
        .expect("build oauth responses request");

        assert_eq!(
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer oauth-upstream-token")
        );
        assert_eq!(
            request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        assert!(request.headers().get(header::CONTENT_LENGTH).is_none());
        assert_eq!(
            request
                .headers()
                .get("ChatGPT-Account-Id")
                .and_then(|value| value.to_str().ok()),
            Some("server-account")
        );
        assert_eq!(
            request
                .headers()
                .get("x-openai-prompt-cache-key")
                .and_then(|value| value.to_str().ok()),
            Some("prompt-cache-gamma")
        );
    }

    #[test]
    fn copy_forwardable_headers_fingerprints_allowlisted_header_values() {
        let client = Client::new();
        let crypto_key: [u8; 32] = Sha256::digest(b"oauth-debug-test-secret").into();
        let headers = HeaderMap::from_iter([
            (
                header::HeaderName::from_static("session_id"),
                "session-alpha".parse().expect("session header"),
            ),
            (
                header::HeaderName::from_static("traceparent"),
                "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"
                    .parse()
                    .expect("traceparent"),
            ),
        ]);

        let (_, summary_a) = copy_forwardable_headers(
            client.get("https://example.com"),
            &headers,
            &[],
            Some(&crypto_key),
        );
        let (_, summary_b) = copy_forwardable_headers(
            client.get("https://example.com"),
            &headers,
            &[],
            Some(&crypto_key),
        );

        let mut changed_headers = headers.clone();
        changed_headers.insert(
            header::HeaderName::from_static("session_id"),
            "session-beta".parse().expect("updated session header"),
        );
        let (_, summary_c) = copy_forwardable_headers(
            client.get("https://example.com"),
            &changed_headers,
            &[],
            Some(&crypto_key),
        );

        assert_eq!(summary_a.fingerprints, summary_b.fingerprints);
        assert_ne!(summary_a.fingerprints, summary_c.fingerprints);
        assert_eq!(
            summary_a
                .fingerprints
                .as_ref()
                .and_then(|fingerprints| fingerprints.get("session_id"))
                .map(String::len),
            Some(16)
        );
        assert_eq!(
            summary_a
                .fingerprints
                .as_ref()
                .and_then(|fingerprints| fingerprints.get("traceparent"))
                .map(String::len),
            Some(16)
        );
    }

    #[test]
    fn build_oauth_request_debug_fingerprints_body_prefix_and_downgrades_without_crypto_key() {
        let crypto_key: [u8; 32] = Sha256::digest(b"oauth-debug-test-secret").into();
        let forwarded_headers = OauthForwardedHeaderSummary {
            names: vec!["session_id".to_string()],
            prompt_cache_header_forwarded: false,
            fingerprints: Some(BTreeMap::from([(
                "session_id".to_string(),
                "0123456789abcdef".to_string(),
            )])),
        };
        let debug = build_oauth_request_debug(
            "/v1/responses",
            &forwarded_headers,
            Some(br#"{"model":"gpt-5.4","input":"hello"}"#),
            OauthResponsesRewriteSummary::default(),
            Some("memory"),
            Some("small_body_rewrite"),
            Some(&crypto_key),
        );
        let no_crypto = build_oauth_request_debug(
            "/v1/responses",
            &forwarded_headers,
            Some(br#"{"model":"gpt-5.4","input":"hello"}"#),
            OauthResponsesRewriteSummary::default(),
            Some("memory"),
            Some("small_body_rewrite"),
            None,
        );

        assert_eq!(debug.fingerprint_version, Some("v1"));
        assert!(
            debug
                .request_body_prefix_bytes
                .expect("body prefix byte count")
                > 0
        );
        assert_eq!(
            debug
                .request_body_prefix_fingerprint
                .as_ref()
                .map(String::len),
            Some(16)
        );
        assert!(no_crypto.fingerprint_version.is_none());
        assert!(no_crypto.request_body_prefix_fingerprint.is_none());
        assert!(no_crypto.request_body_prefix_bytes.is_none());
        assert!(no_crypto.forwarded_header_fingerprints.is_none());
        assert_eq!(debug.request_body_snapshot_kind, Some("memory"));
        assert_eq!(debug.responses_body_mode, Some("small_body_rewrite"));
    }

    #[test]
    fn bytes_response_from_headers_strips_transfer_encoding_on_buffered_body() {
        let headers = HeaderMap::from_iter([
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (
                header::TRANSFER_ENCODING,
                HeaderValue::from_static("chunked"),
            ),
            (header::CONNECTION, HeaderValue::from_static("keep-alive")),
        ]);

        let response = bytes_response_from_headers(
            StatusCode::OK,
            &headers,
            Bytes::from_static(br#"{"ok":true}"#),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        assert!(response.headers().get(header::TRANSFER_ENCODING).is_none());
        assert!(response.headers().get(header::CONNECTION).is_none());
    }

    #[tokio::test]
    async fn oauth_responses_buffered_json_success_passthroughs_non_sse_payload() {
        async fn oauth_json_upstream() -> Response {
            (
                StatusCode::OK,
                [(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/json"),
                )],
                Json(json!({
                    "id": "resp_json_123",
                    "status": "completed",
                    "output_text": "hello"
                })),
            )
                .into_response()
        }

        let _guard = TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK.lock().await;
        let app = Router::new().route("/backend-api/codex/responses", post(oauth_json_upstream));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind oauth json upstream");
        let addr = listener.local_addr().expect("oauth json upstream addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("oauth json upstream should run");
        });
        set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("http://{addr}/backend-api/codex"))
                .expect("valid oauth upstream base url"),
        )
        .await;

        let uri = "/v1/responses".parse().expect("valid uri");
        let headers = HeaderMap::from_iter([(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        )]);
        let oauth_response = send_counted_oauth_upstream_request(
            CountedOauthUpstreamRequestContext {
                request: OauthUpstreamRequestContext {
                    method: Method::POST,
                    original_uri: &uri,
                    headers: &headers,
                    handshake_timeout: Duration::from_secs(5),
                    response_timeout: Duration::from_secs(5),
                    account_id: Some(7),
                    access_token: "oauth-json",
                    chatgpt_account_id: Some("org_test"),
                    installation_seed: Some(&[0x33_u8; 32]),
                    crypto_key: None,
                },
                forward_proxy_url: None,
                reporter: None,
            },
            CountedOauthUpstreamRequestBody::Bytes(Bytes::from_static(
                br#"{"model":"gpt-5.4","stream":false,"input":"hello"}"#,
            )),
        )
        .await;

        assert_eq!(oauth_response.response.status(), StatusCode::OK);
        assert_eq!(
            oauth_response
                .response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/json")
        );
        let body = to_bytes(oauth_response.response.into_body(), usize::MAX)
            .await
            .expect("read oauth json response");
        let payload: Value =
            serde_json::from_slice(&body).expect("decode oauth json response payload");
        assert_eq!(payload["id"], "resp_json_123");
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["output_text"], "hello");

        handle.abort();
        reset_test_oauth_codex_upstream_base_url().await;
    }
}
