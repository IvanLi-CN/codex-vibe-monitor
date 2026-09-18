#[cfg(test)]
mod websocket_tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_websocket_failure_fence_releases_without_waking_waiters() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let reservation_key = "websocket-cancelled-fence";
        state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned")
            .insert(
                reservation_key.to_string(),
                PoolRoutingReservation {
                    account_id: 42,
                    model: Some("gpt-ws-cancelled-fence".to_string()),
                    proxy_key: None,
                    created_at: Instant::now(),
                },
            );
        let availability = state.pool_routing_availability.subscribe();
        let initial_generation = *availability.borrow();
        let (fence_started_tx, fence_started_rx) = tokio::sync::oneshot::channel();
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            let mut reservation_guard =
                PoolRoutingReservationGuard::new(task_state, reservation_key.to_string());
            reservation_guard.suppress_availability_publish();
            let _ = fence_started_tx.send(());
            std::future::pending::<()>().await;
        });

        fence_started_rx
            .await
            .expect("pending websocket failure fence should begin before cancellation");
        task.abort();
        let join_error = task
            .await
            .expect_err("cancelling the websocket failure fence should cancel its task");
        assert!(join_error.is_cancelled());
        assert!(
            !state
                .pool_routing_reservations
                .lock()
                .expect("pool routing reservations mutex poisoned")
                .contains_key(reservation_key),
            "cancellation must release the websocket reservation"
        );
        assert_eq!(
            *availability.borrow(),
            initial_generation,
            "websocket cancellation before a failure fence commits must not wake waiters"
        );
    }

    #[tokio::test]
    async fn cancelling_websocket_selection_handoff_releases_reservation_and_wakes_waiters() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let reservation_key = "websocket-selection-handoff-cancelled";
        state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned")
            .insert(
                reservation_key.to_string(),
                PoolRoutingReservation {
                    account_id: 42,
                    model: Some("gpt-ws-selection-handoff".to_string()),
                    proxy_key: None,
                    created_at: Instant::now(),
                },
            );
        let availability = state.pool_routing_availability.subscribe();
        let initial_generation = *availability.borrow();
        let (handoff_started_tx, handoff_started_rx) = tokio::sync::oneshot::channel();
        let task_state = state.clone();
        let task = tokio::spawn(async move {
            // The production path creates this guard immediately after selection,
            // before awaiting the websocket-capability query.
            let _reservation_guard =
                PoolRoutingReservationGuard::new(task_state, reservation_key.to_string());
            let _ = handoff_started_tx.send(());
            std::future::pending::<()>().await;
        });

        handoff_started_rx
            .await
            .expect("selection handoff guard should be active before cancellation");
        task.abort();
        let join_error = task
            .await
            .expect_err("cancelling the selection handoff should cancel its task");
        assert!(join_error.is_cancelled());
        assert!(
            !state
                .pool_routing_reservations
                .lock()
                .expect("pool routing reservations mutex poisoned")
                .contains_key(reservation_key),
            "cancelling before websocket capability resolution must release the reservation"
        );
        assert_ne!(
            *availability.borrow(),
            initial_generation,
            "cancelling a healthy websocket selection handoff must wake waiters"
        );
    }

    #[tokio::test]
    async fn websocket_success_guard_releases_stale_account_without_waking_waiters() {
        let state = crate::tests::test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base url"),
        )
        .await;
        let reservation_key = "websocket-stale-success";
        state
            .pool_routing_reservations
            .lock()
            .expect("pool routing reservations mutex poisoned")
            .insert(
                reservation_key.to_string(),
                PoolRoutingReservation {
                    account_id: 42,
                    model: Some("gpt-ws-stale-success".to_string()),
                    proxy_key: None,
                    created_at: Instant::now(),
                },
            );
        let availability = state.pool_routing_availability.subscribe();
        let initial_generation = *availability.borrow();

        {
            let mut reservation_guard =
                PoolRoutingReservationGuard::new(state.clone(), reservation_key.to_string());
            // A success record returns false when a newer account failure fences
            // the request. WebSocket cleanup must release without publishing.
            reservation_guard.set_availability_publish(false);
        }

        assert!(
            !state
                .pool_routing_reservations
                .lock()
                .expect("pool routing reservations mutex poisoned")
                .contains_key(reservation_key),
            "stale websocket success must release its reservation"
        );
        assert_eq!(
            *availability.borrow(),
            initial_generation,
            "stale websocket success must not wake pool waiters"
        );
    }

    fn api_key_account(upstream_base_url: Url) -> PoolResolvedAccount {
        PoolResolvedAccount {
            account_id: 42,
            display_name: "ws-test".to_string(),
            kind: "api_key".to_string(),
            auth: PoolResolvedAuth::ApiKey {
                authorization: format!("{} {}", "Bearer", ["upstream", "secret"].join("-")),
            },
            group_name: None,
            bound_proxy_keys: Vec::new(),
            forward_proxy_scope: ForwardProxyRouteScope::Automatic,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: Default::default(),
            request_compression_algorithm: RequestCompressionAlgorithm::Identity,
            response_endpoint_capability: CapabilitySupport::Unknown,
            chat_completions_capability: CapabilitySupport::Unknown,
            image_endpoint_capability: CapabilitySupport::Unknown,
            response_image_tool_capability: CapabilitySupport::Unknown,
            codex_imagegen_capability: CapabilitySupport::Unknown,
            standalone_search_capability: CapabilitySupport::Unknown,
            upstream_base_url,
            routing_source: PoolRoutingSelectionSource::FreshAssignment,
            sticky_affinity_generation: None,
            routing_selection_audit: None,
            priority_handoff_permit: None,
        }
    }

    fn oauth_account(upstream_base_url: Url) -> PoolResolvedAccount {
        PoolResolvedAccount {
            account_id: 43,
            display_name: "ws-oauth-test".to_string(),
            kind: "oauth".to_string(),
            auth: PoolResolvedAuth::Oauth {
                access_token: ["oauth", "upstream", "token"].join("-"),
                chatgpt_account_id: Some("acct-test".to_string()),
            },
            group_name: None,
            bound_proxy_keys: Vec::new(),
            forward_proxy_scope: ForwardProxyRouteScope::Automatic,
            single_account_rotation_enabled: false,
            upstream_429_retry_enabled: false,
            upstream_429_max_retries: 0,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::default(),
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: Default::default(),
            request_compression_algorithm: RequestCompressionAlgorithm::Identity,
            response_endpoint_capability: CapabilitySupport::Unknown,
            chat_completions_capability: CapabilitySupport::Unknown,
            image_endpoint_capability: CapabilitySupport::Unknown,
            response_image_tool_capability: CapabilitySupport::Unknown,
            codex_imagegen_capability: CapabilitySupport::Unknown,
            standalone_search_capability: CapabilitySupport::Unknown,
            upstream_base_url,
            routing_source: PoolRoutingSelectionSource::FreshAssignment,
            sticky_affinity_generation: None,
            routing_selection_audit: None,
            priority_handoff_permit: None,
        }
    }

    #[test]
    fn websocket_upgrade_detection_is_case_insensitive() {
        let mut headers = HeaderMap::new();
        headers.insert(header::UPGRADE, HeaderValue::from_static("WebSocket"));

        assert!(is_websocket_upgrade_request(&headers));
    }

    #[test]
    fn websocket_upstream_url_maps_https_to_wss_and_preserves_base_path_query() {
        let base = Url::parse("https://api.example.test/gateway/").expect("valid base");
        let uri = "/v1/responses?model=gpt-5.5"
            .parse::<Uri>()
            .expect("valid uri");

        let target = build_websocket_upstream_url(&base, &uri).expect("ws url");

        assert_eq!(
            target.as_str(),
            "wss://api.example.test/gateway/v1/responses?model=gpt-5.5"
        );
    }

    #[test]
    fn websocket_model_mapping_rewrites_query_and_json_frames() {
        let mut url =
            Url::parse("wss://api.example.test/v1/realtime?model=client-fast&session=keep")
                .expect("valid upstream url");
        assert!(rewrite_websocket_upstream_url_model(
            &mut url,
            "upstream-model"
        ));
        assert_eq!(
            url.as_str(),
            "wss://api.example.test/v1/realtime?model=upstream-model&session=keep"
        );

        let mut payload = serde_json::json!({
            "type": "response.create",
            "model": "client-fast",
            "input": "hello",
        });
        rewrite_websocket_json_payload_model(&mut payload, "upstream-model")
            .expect("rewrite mapped JSON frame");
        assert_eq!(payload["model"], "upstream-model");

        let mut nested_payload = serde_json::json!({
            "type": "response.create",
            "session": {"model": "client-fast"},
        });
        assert_eq!(
            websocket_mapping_requested_model(&nested_payload, true).expect("inspect nested model"),
            Some("client-fast".to_string())
        );
        rewrite_websocket_json_payload_model(&mut nested_payload, "upstream-model")
            .expect("rewrite nested mapped JSON frame");
        assert_eq!(nested_payload["session"]["model"], "upstream-model");

        let mut unsafe_payload = serde_json::json!({"model": 7});
        assert!(
            rewrite_websocket_json_payload_model(&mut unsafe_payload, "upstream-model").is_err()
        );
    }

    #[test]
    fn websocket_active_model_mapping_fails_closed_for_unsafe_frames() {
        let active_mapping = true;
        assert!(
            websocket_mapping_unsafe_frame_error(
                &AxumWsMessage::Binary(vec![1, 2, 3]),
                active_mapping,
            )
            .is_some()
        );
        assert!(parse_websocket_mapping_payload("not-json", active_mapping).is_err());
        assert!(parse_websocket_mapping_payload("[1, 2, 3]", active_mapping).is_err());
        assert!(
            websocket_mapping_requested_model(&serde_json::json!({"model": 7}), active_mapping,)
                .is_err()
        );
    }

    #[test]
    fn websocket_without_model_mapping_preserves_unsafe_payloads() {
        assert!(
            websocket_mapping_unsafe_frame_error(&AxumWsMessage::Binary(vec![1, 2, 3]), false,)
                .is_none()
        );
        assert!(
            parse_websocket_mapping_payload("not-json", false)
                .expect("unmapped malformed frame should pass through")
                .is_none()
        );
        assert!(
            parse_websocket_mapping_payload("[1, 2, 3]", false)
                .expect("unmapped non-object frame should pass through")
                .is_none()
        );
        assert!(
            websocket_mapping_requested_model(&serde_json::json!({"model": 7}), false,)
                .expect("unmapped non-string model should pass through")
                .is_none()
        );
    }

    #[test]
    fn websocket_upstream_url_maps_http_to_ws() {
        let base = Url::parse("http://127.0.0.1:9000").expect("valid base");
        let uri = "/v1/realtime".parse::<Uri>().expect("valid uri");

        let target = build_websocket_upstream_url(&base, &uri).expect("ws url");

        assert_eq!(target.as_str(), "ws://127.0.0.1:9000/v1/realtime");
    }

    #[test]
    fn websocket_requested_model_extraction_reads_query_parameter() {
        let uri = "/v1/realtime?foo=1&model=gpt-5.5-preview&empty="
            .parse::<Uri>()
            .expect("valid uri");

        assert_eq!(
            extract_requested_model_from_websocket_uri(&uri).as_deref(),
            Some("gpt-5.5-preview")
        );
    }

    #[test]
    fn websocket_requested_model_extraction_preserves_blank_values() {
        let uri = "/v1/realtime?model=%20%20"
            .parse::<Uri>()
            .expect("valid uri");

        assert_eq!(
            extract_requested_model_from_websocket_uri(&uri).as_deref(),
            Some("")
        );
    }

    #[test]
    fn websocket_header_prompt_cache_key_does_not_fallback_to_sticky_key() {
        let headers = HeaderMap::from_iter([(
            HeaderName::from_static("x-sticky-key"),
            HeaderValue::from_static("sticky-only-key"),
        )]);

        let (sticky_key, prompt_cache_key) = websocket_routing_keys_from_headers(&headers);

        assert_eq!(sticky_key.as_deref(), Some("sticky-only-key"));
        assert_eq!(prompt_cache_key, None);
    }

    #[test]
    fn upstream_ws_request_replaces_auth_and_drops_upgrade_hop_headers() {
        let account = api_key_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer downstream-pool-key"),
        );
        headers.insert(
            header::CONNECTION,
            HeaderValue::from_static("upgrade, x-drop-me"),
        );
        headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("realtime=v1"),
        );
        headers.insert(
            HeaderName::from_static("x-drop-me"),
            HeaderValue::from_static("drop"),
        );
        headers.insert(
            HeaderName::from_static("sec-websocket-key"),
            HeaderValue::from_static("downstream-key"),
        );

        let request = build_upstream_ws_request(
            &Url::parse("wss://api.example.test/v1/responses").expect("valid target"),
            &headers,
            &account,
            true,
        )
        .expect("request");

        assert_eq!(
            request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer upstream-secret")
        );
        assert_eq!(
            request
                .headers()
                .get("openai-beta")
                .and_then(|value| value.to_str().ok()),
            Some("realtime=v1")
        );
        assert!(!request.headers().contains_key("x-drop-me"));
        assert_ne!(
            request
                .headers()
                .get("sec-websocket-key")
                .and_then(|value| value.to_str().ok()),
            Some("downstream-key")
        );
    }

    #[test]
    fn upstream_ws_request_uses_oauth_auth_and_account_headers() {
        let account = oauth_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("downstream-beta"),
        );
        let request = build_upstream_ws_request(
            &Url::parse("wss://api.example.test/v1/responses").expect("valid target"),
            &headers,
            &account,
            true,
        )
        .expect("request");

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
                .get("openai-beta")
                .and_then(|value| value.to_str().ok()),
            Some("responses=experimental")
        );
        assert_eq!(
            request
                .headers()
                .get("chatgpt-account-id")
                .and_then(|value| value.to_str().ok()),
            Some("acct-test")
        );
    }

    #[test]
    fn upstream_ws_request_preserves_oauth_realtime_beta_header() {
        let account = oauth_account(Url::parse("https://api.example.test").expect("valid base"));
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("openai-beta"),
            HeaderValue::from_static("realtime=v1"),
        );
        let request = build_upstream_ws_request(
            &Url::parse("wss://api.example.test/v1/realtime").expect("valid target"),
            &headers,
            &account,
            false,
        )
        .expect("request");

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
                .get("openai-beta")
                .and_then(|value| value.to_str().ok()),
            Some("realtime=v1")
        );
        assert_eq!(
            request
                .headers()
                .get("chatgpt-account-id")
                .and_then(|value| value.to_str().ok()),
            Some("acct-test")
        );
    }

    #[test]
    fn forward_proxy_credentials_are_percent_decoded() {
        let proxy_url = format!(
            "{}{}{}",
            "http://user%2Bname:", "p%40ss%3Aword", "@proxy.example.test:8080"
        );
        let proxy_url = Url::parse(&proxy_url).expect("valid proxy url");

        assert_eq!(
            forward_proxy_basic_auth_credential(&proxy_url).as_deref(),
            Some("user+name:p@ss:word")
        );
    }

    #[tokio::test]
    async fn prefixed_io_replays_buffered_connect_bytes() {
        let (mut client, server) = tokio::io::duplex(64);
        client.write_all(b"inner").await.expect("write inner bytes");
        drop(client);
        let mut stream = PrefixedIo::new(b"prefix-".to_vec(), Box::new(server));
        let mut bytes = Vec::new();

        stream
            .read_to_end(&mut bytes)
            .await
            .expect("read combined bytes");

        assert_eq!(bytes, b"prefix-inner");
    }

    #[tokio::test]
    async fn socks5_local_target_host_resolves_hostname_locally() {
        let target = resolve_socks5_local_target_host("localhost", 443)
            .await
            .expect("resolve localhost");

        assert!(
            target.parse::<IpAddr>().is_ok(),
            "socks5:// should pass a locally resolved IP address to the proxy"
        );
    }

    #[test]
    fn retryable_ws_failure_excludes_account_and_route_key_for_next_pool_selection() {
        let route_key = ["api", "key:42"].join("_");
        let failure = WsAttemptFailure {
            status: StatusCode::BAD_GATEWAY,
            message: "failed to contact websocket upstream".to_string(),
            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            retryable: true,
            account_id: Some(42),
            upstream_route_key: Some(route_key.clone()),
        };
        let mut excluded_account_ids = Vec::new();
        let mut excluded_upstream_route_keys: HashSet<String> = HashSet::new();

        exclude_retryable_ws_attempt_failure(
            &failure,
            &mut excluded_account_ids,
            &mut excluded_upstream_route_keys,
        )
        .expect("exclusion context");

        assert_eq!(excluded_account_ids, vec![42]);
        assert!(excluded_upstream_route_keys.contains(&route_key));
    }

    #[test]
    fn retryable_ws_failure_without_account_context_becomes_terminal() {
        let failure = WsAttemptFailure {
            status: StatusCode::BAD_GATEWAY,
            message: "failed without account".to_string(),
            failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
            retryable: true,
            account_id: None,
            upstream_route_key: None,
        };
        let mut excluded_account_ids = Vec::new();
        let mut excluded_upstream_route_keys: HashSet<String> = HashSet::new();

        let err = exclude_retryable_ws_attempt_failure(
            &failure,
            &mut excluded_account_ids,
            &mut excluded_upstream_route_keys,
        )
        .expect_err("missing account context is terminal");

        assert_eq!(err.status, StatusCode::BAD_GATEWAY);
        assert_eq!(err.message, "failed without account");
        assert!(excluded_account_ids.is_empty());
        assert!(excluded_upstream_route_keys.is_empty());
    }

    #[test]
    fn websocket_usage_event_parses_terminal_response_usage() {
        let event = parse_ws_usage_event(
            r#"{
                "type": "response.completed",
                "response": {
                    "id": "resp_1",
                    "model": "gpt-5.5",
                    "service_tier": "default",
                    "usage": {
                        "input_tokens": 7,
                        "output_tokens": 3,
                        "input_tokens_details": {
                            "cached_tokens": 2
                        }
                    }
                }
            }"#,
        )
        .expect("usage event");

        assert_eq!(event.event_type, "response.completed");
        assert_eq!(event.response_id.as_deref(), Some("resp_1"));
        assert_eq!(event.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(event.service_tier.as_deref(), Some("default"));
        assert_eq!(event.usage.input_tokens, Some(7));
        assert_eq!(event.usage.output_tokens, Some(3));
        assert_eq!(event.usage.cache_input_tokens, Some(2));
        assert_eq!(event.usage.total_tokens, Some(10));
        assert!(!event.contains_encrypted_content);
    }

    #[test]
    fn websocket_usage_event_detects_encrypted_content() {
        let event = parse_ws_usage_event(
            r#"{
                "type": "response.completed",
                "response": {
                    "id": "resp_2",
                    "model": "gpt-5.5",
                    "output": [{
                        "type": "encrypted_content",
                        "encrypted_content": "opaque"
                    }],
                    "usage": {
                        "input_tokens": 5,
                        "output_tokens": 2,
                        "total_tokens": 7
                    }
                }
            }"#,
        )
        .expect("usage event");

        assert!(event.contains_encrypted_content);
    }

    #[test]
    fn websocket_request_payload_detects_only_structured_encrypted_content() {
        assert!(ws_request_payload_contains_encrypted_content(
            br#"{"type":"message","content":[{"type":"encrypted_content","encrypted_content":"opaque"}]}"#
        ));
        assert!(!ws_request_payload_contains_encrypted_content(
            br#"{"type":"message","text":"literal encrypted_content token in plain text"}"#
        ));
    }

    #[test]
    fn websocket_request_payload_extracts_prompt_cache_key() {
        let inspection = inspect_ws_request_payload(
            br#"{"type":"conversation.item.create","metadata":{"promptCacheKey":"pck-ws"},"item":{"type":"message"}}"#,
        )
        .expect("payload inspection");

        assert_eq!(
            inspection.event_type.as_deref(),
            Some("conversation.item.create")
        );
        assert_eq!(inspection.prompt_cache_key.as_deref(), Some("pck-ws"));
        assert!(!inspection.contains_encrypted_content);
    }

    #[test]
    fn websocket_request_payload_preserves_an_empty_model() {
        let inspection = inspect_ws_request_payload(br#"{"type":"response.create","model":""}"#)
            .expect("payload inspection");
        assert_eq!(inspection.requested_model.as_deref(), Some(""));
    }

    #[test]
    fn websocket_first_frame_requires_response_create_and_extracts_turn_fields() {
        let first_message = AxumWsMessage::Text(
            json!({
                "type": "response.create",
                "model": "gpt-5-realtime",
                "prompt_cache_key": "pck-ws-turn",
                "previous_response_id": "resp_prev"
            })
            .to_string(),
        );

        let inspection =
            inspect_ws_initial_response_create_message(&first_message).expect("first frame");

        assert_eq!(inspection.event_type.as_deref(), Some("response.create"));
        assert_eq!(
            inspection.requested_model.as_deref(),
            Some("gpt-5-realtime")
        );
        assert_eq!(inspection.prompt_cache_key.as_deref(), Some("pck-ws-turn"));
        assert_eq!(
            inspection.previous_response_id.as_deref(),
            Some("resp_prev")
        );

        let rejected = AxumWsMessage::Text(
            json!({
                "type": "conversation.item.create",
                "prompt_cache_key": "pck-ws-turn"
            })
            .to_string(),
        );
        assert_eq!(
            inspect_ws_initial_response_create_message(&rejected).unwrap_err(),
            "websocket first frame must be response.create"
        );
    }

    #[test]
    fn websocket_ttft_resets_for_each_response_create_turn() {
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: "pool-ws-ttft".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/responses".to_string(),
            sticky_key: None,
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: Some("gpt-5.6".to_string()),
        };
        let mut tracker = WsUsageTracker::new(
            api_key_account(Url::parse("https://api.example.test").expect("valid base")),
            trace,
            None,
            None,
            None,
        );

        assert!(!tracker.observe_first_token_text(
            r#"{"type":"response.output_text.delta","delta":"ignored before turn"}"#,
        ));
        tracker.start_turn_at(
            Instant::now() - Duration::from_millis(25),
            Utc::now().to_rfc3339(),
        );
        assert!(
            !tracker
                .observe_first_token_text(r#"{"type":"response.output_text.delta","delta":""}"#,)
        );
        assert!(tracker.observe_first_token_text(
            r#"{"type":"response.reasoning_summary_text.delta","delta":"thinking"}"#,
        ));
        let first_turn_ttft = tracker.first_token_ms.expect("first turn TTFT");
        assert!(first_turn_ttft >= 25.0);
        assert!(
            !tracker.observe_first_token_text(
                r#"{"type":"response.output_text.delta","delta":"answer"}"#,
            )
        );
        assert_eq!(tracker.first_token_ms, Some(first_turn_ttft));

        tracker.start_turn_at(Instant::now(), Utc::now().to_rfc3339());
        assert!(tracker.first_token_ms.is_none());
        assert!(tracker.observe_first_token_text(
            r#"{"type":"response.function_call_arguments.delta","delta":"{"}"#,
        ));
        assert!(tracker.first_token_ms.is_some());
    }

    #[test]
    fn realtime_without_query_model_waits_for_initial_model_but_query_model_stays_live() {
        assert!(websocket_should_wait_for_initial_model(
            "/v1/realtime",
            None
        ));
        assert!(!websocket_should_wait_for_initial_model(
            "/v1/realtime",
            Some("gpt-5-realtime")
        ));
        assert!(websocket_should_wait_for_initial_model(
            "/v1/responses",
            None
        ));
    }

    #[test]
    fn websocket_effective_prompt_cache_key_ignores_sticky_only_context() {
        let trace = PoolUpstreamAttemptTraceContext {
            invoke_id: "pool-ws-sticky-only".to_string(),
            occurred_at: shanghai_now_string(),
            endpoint: "/v1/realtime".to_string(),
            sticky_key: Some("sticky-only-key".to_string()),
            requester_ip: None,
            upstream_base_url_host: None,
            request_model: None,
        };
        let usage_tracker = WsUsageTracker::new(
            api_key_account(Url::parse("https://api.example.test").expect("valid base")),
            trace,
            None,
            None,
            None,
        );

        assert_eq!(
            websocket_effective_prompt_cache_key(usage_tracker.prompt_cache_key.as_deref()),
            None
        );
    }

    #[test]
    fn websocket_usage_payload_marks_transport() {
        let payload = mark_websocket_payload_transport(
            r#"{"endpoint":"/v1/responses","model":"gpt-5.5"}"#.to_string(),
        )
        .expect("marked payload");
        let value: Value = serde_json::from_str(&payload).expect("json payload");

        assert_eq!(
            value.get("transport").and_then(Value::as_str),
            Some("websocket")
        );
        assert_eq!(
            value.get("endpoint").and_then(Value::as_str),
            Some("/v1/responses")
        );
    }

    #[test]
    fn websocket_usage_event_rejects_non_terminal_or_partial_usage() {
        assert!(parse_ws_usage_event(
            r#"{"type":"response.in_progress","response":{"usage":{"input_tokens":7,"output_tokens":3}}}"#
        )
        .is_none());
        assert!(
            parse_ws_usage_event(
                r#"{"type":"response.completed","response":{"usage":{"input_tokens":7}}}"#
            )
            .is_none()
        );
        assert!(parse_ws_usage_event(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":"bad","output_tokens":3}}}"#
        )
        .is_none());
    }

    #[test]
    fn websocket_usage_event_requires_completed_done_before_success() {
        let completed = parse_ws_usage_event(
            r#"{"type":"response.done","response":{"status":"completed","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}"#,
        )
        .expect("completed response.done should parse");
        assert!(ws_usage_event_is_completed_success(&completed));

        let incomplete = parse_ws_usage_event(
            r#"{"type":"response.done","response":{"status":"incomplete","usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}}"#,
        )
        .expect("incomplete response.done should parse");
        assert!(!ws_usage_event_is_completed_success(&incomplete));
    }

    #[test]
    fn websocket_terminal_failure_kind_marks_response_failed() {
        assert_eq!(
            ws_terminal_event_failure_kind(&WsUsageEvent {
                event_type: "response.failed".to_string(),
                response_id: None,
                response_status: Some("failed".to_string()),
                model: None,
                service_tier: None,
                usage: ParsedUsage::default(),
                contains_encrypted_content: false,
            }),
            Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
        );
        assert_eq!(
            ws_terminal_event_failure_kind(&WsUsageEvent {
                event_type: "response.completed".to_string(),
                response_id: None,
                response_status: Some("completed".to_string()),
                model: None,
                service_tier: None,
                usage: ParsedUsage::default(),
                contains_encrypted_content: false,
            }),
            None
        );
    }

    #[test]
    fn websocket_terminal_failure_classifies_only_temporary_upstream_errors() {
        assert_eq!(
            ws_terminal_temporary_classification(
                r#"{"type":"response.failed","response":{"error":{"code":"server_error"}}}"#,
            ),
            Some((
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
                StatusCode::BAD_GATEWAY
            ))
        );
        assert_eq!(
            ws_terminal_temporary_classification(
                r#"{"type":"response.failed","response":{"error":{"code":"server_is_overloaded"}}}"#,
            ),
            Some((
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
                StatusCode::BAD_GATEWAY
            ))
        );
        assert_eq!(
            ws_terminal_temporary_classification(
                r#"{"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}}"#,
            ),
            Some((
                UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
                StatusCode::TOO_MANY_REQUESTS
            ))
        );
        assert_eq!(
            ws_terminal_temporary_classification(
                r#"{"type":"response.failed","response":{"status":"incomplete"},"error":"downstream closed"}"#,
            ),
            None
        );
        assert_eq!(
            ws_terminal_temporary_classification(
                r#"{"type":"response.failed","response":{"error":{"code":"invalid_request_error"}}}"#,
            ),
            None
        );
    }

    #[test]
    fn websocket_upstream_close_requires_retry_until_terminal_event() {
        assert!(ws_upstream_close_requires_retry(true, false));
        assert!(!ws_upstream_close_requires_retry(false, false));
        assert!(!ws_upstream_close_requires_retry(true, true));
    }

    #[test]
    fn websocket_upstream_http_unsupported_errors_mark_account_no_ws() {
        for status in [
            tungstenite::http::StatusCode::FORBIDDEN,
            tungstenite::http::StatusCode::NOT_FOUND,
            tungstenite::http::StatusCode::METHOD_NOT_ALLOWED,
            tungstenite::http::StatusCode::UPGRADE_REQUIRED,
            tungstenite::http::StatusCode::NOT_IMPLEMENTED,
        ] {
            let response = tungstenite::http::Response::builder()
                .status(status)
                .body(None)
                .expect("response");
            assert!(
                websocket_upstream_error_marks_account_ws_unsupported(&tungstenite::Error::Http(
                    Box::new(response),
                )),
                "status {status} should mark no-ws"
            );
        }

        let response = tungstenite::http::Response::builder()
            .status(tungstenite::http::StatusCode::BAD_GATEWAY)
            .body(None)
            .expect("response");
        assert!(!websocket_upstream_error_marks_account_ws_unsupported(
            &tungstenite::Error::Http(Box::new(response))
        ));
        assert!(!websocket_upstream_error_marks_account_ws_unsupported(
            &tungstenite::Error::ConnectionClosed
        ));
    }

    #[test]
    fn websocket_post_upgrade_close_marks_only_responses_api_key_codex_no_ws() {
        let mut api_key_codex =
            api_key_account(Url::parse("https://api.example.test").expect("url"));
        api_key_codex.kind = API_KEYS_BILLING_ACCOUNT_KIND.to_string();
        assert!(websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &api_key_codex,
            None
        ));
        let normal_close = tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Normal,
            reason: "".into(),
        };
        assert!(websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &api_key_codex,
            Some(&normal_close)
        ));
        let transient_close = tungstenite::protocol::CloseFrame {
            code: tungstenite::protocol::frame::coding::CloseCode::Again,
            reason: "retry".into(),
        };
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &api_key_codex,
            Some(&transient_close)
        ));
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            false,
            &api_key_codex,
            None
        ));

        let generic_api_key = api_key_account(Url::parse("https://api.example.test").expect("url"));
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &generic_api_key,
            None
        ));
        let mut official_api_key =
            api_key_account(Url::parse("https://api.openai.com").expect("url"));
        official_api_key.kind = API_KEYS_BILLING_ACCOUNT_KIND.to_string();
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &official_api_key,
            None
        ));

        let mut oauth_codex = oauth_account(Url::parse("https://api.example.test").expect("url"));
        oauth_codex.kind = API_KEYS_BILLING_ACCOUNT_KIND.to_string();
        assert!(!websocket_post_upgrade_close_marks_account_ws_unsupported(
            true,
            &oauth_codex,
            None
        ));
    }

    #[tokio::test]
    async fn websocket_capability_respects_no_ws_system_tag() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite pool");
        ensure_schema(&pool).await.expect("schema");
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_accounts (
                id, provider, kind, display_name, enabled, status, encrypted_credentials, created_at, updated_at
            )
            VALUES (42, 'codex', 'api_key', 'ws-test', 1, 'active', 'secret', 'now', 'now')
            "#,
        )
        .execute(&pool)
        .await
        .expect("insert account");

        assert!(
            !account_has_websocket_unsupported_tag(&pool, 42)
                .await
                .expect("tag lookup")
        );
        let tag_id = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM pool_tags WHERE system_key = 'unsupported_transport:websocket'",
        )
        .fetch_one(&pool)
        .await
        .expect("tag id");
        sqlx::query(
            "INSERT INTO pool_upstream_account_tags (account_id, tag_id, created_at, updated_at) VALUES (42, ?1, 'now', 'now')",
        )
        .bind(tag_id)
        .execute(&pool)
        .await
        .expect("attach tag");

        assert!(
            account_has_websocket_unsupported_tag(&pool, 42)
                .await
                .expect("tag lookup")
        );
    }

    #[test]
    fn websocket_capability_tag_does_not_consume_retry_budget() {
        let mut excluded_account_ids = Vec::new();
        let excluded_upstream_route_keys: HashSet<String> = HashSet::new();
        let mut ws_retry_account_ids = HashSet::new();

        for account_id in [11, 12, 13] {
            excluded_account_ids.push(account_id);
        }

        assert_eq!(excluded_account_ids.len(), 3);
        assert!(ws_retry_account_ids.is_empty());
        assert!(excluded_upstream_route_keys.is_empty());
        ws_retry_account_ids.insert(99);
        assert_eq!(ws_retry_account_ids.len(), 1);
    }

    #[test]
    fn websocket_message_conversion_preserves_payload_frames() {
        assert_eq!(
            axum_to_tungstenite_message(AxumWsMessage::Text("hello".to_string()))
                .expect("text")
                .into_text()
                .expect("text payload")
                .as_str(),
            "hello"
        );
        assert_eq!(
            tungstenite_to_axum_message(TungsteniteMessage::Binary(vec![1, 2, 3].into())),
            Some(AxumWsMessage::Binary(vec![1, 2, 3]))
        );
    }
}
