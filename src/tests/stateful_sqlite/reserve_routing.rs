use super::*;
use serde_json::json;

#[derive(Clone)]
struct ReserveFallbackUpstreamState {
    request_models: Arc<std::sync::Mutex<Vec<String>>>,
    request_paths: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn reserve_fallback_upstream(
    State(state): State<ReserveFallbackUpstreamState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    state
        .request_paths
        .lock()
        .expect("lock OAuth request paths")
        .push(uri.path().to_string());
    if method == Method::GET && uri.path().contains("/wham/usage") {
        return (
            StatusCode::OK,
            Json(json!({
                "planType": "team",
                "rateLimit": {
                    "primaryWindow": {
                        "usedPercent": 100,
                        "windowDurationMins": 300,
                        "resetsAt": "2099-03-17T14:00:00Z"
                    },
                    "secondaryWindow": {
                        "usedPercent": 80,
                        "windowDurationMins": 10080,
                        "resetsAt": "2099-03-24T00:00:00Z"
                    }
                },
                "additionalRateLimits": {
                    "gpt-reserve": {
                        "rateLimit": {
                            "allowed": true,
                            "limitReached": false,
                            "primaryWindow": {
                                "usedPercent": 10,
                                "availablePercent": 90,
                                "windowDurationMins": 300,
                                "resetsAt": "2099-03-17T14:00:00Z"
                            },
                            "secondaryWindow": {
                                "usedPercent": 20,
                                "availablePercent": 80,
                                "windowDurationMins": 10080,
                                "resetsAt": "2099-03-24T00:00:00Z"
                            }
                        }
                    }
                }
            })),
        )
            .into_response();
    }

    if method == Method::POST && uri.path().ends_with("/responses") {
        let content_encoding = headers
            .get(http_header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok());
        let decoded = if content_encoding == Some("zstd") {
            zstd::decode_all(body.as_ref()).expect("decode OAuth request body")
        } else {
            body.to_vec()
        };
        let payload: Value = serde_json::from_slice(&decoded).expect("decode OAuth request JSON");
        let model = payload["model"]
            .as_str()
            .expect("OAuth request should include model")
            .to_string();
        state
            .request_models
            .lock()
            .expect("lock OAuth request models")
            .push(model.clone());
        if model == RESERVE_WIRE_MODEL {
            let event = json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_reserve_fallback",
                    "model": RESERVE_WIRE_MODEL,
                    "status": "completed",
                    "usage": {
                        "input_tokens": 2,
                        "output_tokens": 1,
                        "total_tokens": 3
                    }
                }
            });
            return (
                StatusCode::OK,
                [(
                    http_header::CONTENT_TYPE,
                    HeaderValue::from_static("text/event-stream"),
                )],
                format!("event: response.completed\ndata: {event}\n\n"),
            )
                .into_response();
        }
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({
                "error": {
                    "code": "insufficient_quota",
                    "message": "quota exhausted for normal quota"
                }
            })),
        )
            .into_response();
    }

    StatusCode::NOT_FOUND.into_response()
}

fn run_reserve_routing_future_with_large_stack<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    std::thread::Builder::new()
        .name("reserve-routing-large-stack".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build Reserve routing test runtime")
                .block_on(future)
        })
        .expect("spawn Reserve routing test worker")
        .join()
        .expect("join Reserve routing test worker")
}

#[test]
fn automatic_reserve_fallback_retries_same_oauth_account_with_wire_model() {
    run_reserve_routing_future_with_large_stack(async move {
        let _upstream_lock = oauth_bridge::TEST_OAUTH_CODEX_UPSTREAM_BASE_URL_LOCK
            .lock()
            .await;
        let request_models = Arc::new(std::sync::Mutex::new(Vec::new()));
        let request_paths = Arc::new(std::sync::Mutex::new(Vec::new()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Reserve fallback upstream");
        let address = listener
            .local_addr()
            .expect("Reserve fallback upstream address");
        let upstream_handle = tokio::spawn({
            let state = ReserveFallbackUpstreamState {
                request_models: request_models.clone(),
                request_paths: request_paths.clone(),
            };
            async move {
                axum::serve(
                    listener,
                    Router::new()
                        .fallback(reserve_fallback_upstream)
                        .with_state(state),
                )
                .await
                .expect("serve Reserve fallback upstream");
            }
        });
        oauth_bridge::set_test_oauth_codex_upstream_base_url(
            Url::parse(&format!("http://{address}/backend-api/codex"))
                .expect("valid OAuth upstream URL"),
        )
        .await;

        let mut state = test_state_with_openai_base(
            Url::parse("https://api.openai.com/").expect("valid upstream base URL"),
        )
        .await;
        Arc::get_mut(&mut state)
            .expect("Reserve routing test state should be uniquely owned")
            .config
            .upstream_accounts_usage_base_url =
            Url::parse(&format!("http://{address}/backend-api")).expect("valid usage base URL");
        seed_pool_routing_api_key(&state, "pool-live-key").await;
        let account_id = insert_test_pool_oauth_account(
            &state,
            "Reserve fallback OAuth",
            "oauth-reserve-fallback",
        )
        .await;

        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri("/v1/responses".parse().expect("valid URI")),
            Method::POST,
            HeaderMap::from_iter([(
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-live-key"),
            )]),
            Body::from(br#"{"model":"gpt-5.4-luna","input":"hello"}"#.to_vec()),
        )
        .await;

        let response_status = response.status();
        let response_body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read Reserve fallback response");
        let captured_models = request_models
            .lock()
            .expect("lock captured OAuth request models")
            .clone();
        let captured_paths = request_paths
            .lock()
            .expect("lock captured OAuth request paths")
            .clone();
        assert_eq!(
            response_status,
            StatusCode::OK,
            "unexpected response body={}, captured models={captured_models:?}, paths={captured_paths:?}",
            String::from_utf8_lossy(&response_body),
        );
        assert!(
            String::from_utf8_lossy(&response_body).contains("resp_reserve_fallback"),
            "Reserve fallback response should reach the client"
        );

        assert_eq!(captured_models, ["gpt-5.4-luna", RESERVE_WIRE_MODEL]);
        assert_eq!(
            captured_paths,
            [
                "/backend-api/codex/responses",
                "/backend-api/wham/usage",
                "/backend-api/codex/responses",
            ]
        );

        let latest_sample = load_latest_usage_sample(&state.pool, account_id)
            .await
            .expect("load refreshed usage sample")
            .expect("refreshed usage sample should exist");
        assert_eq!(
            latest_sample.reserve_status.as_deref(),
            Some(RESERVE_AVAILABILITY_AVAILABLE)
        );
        assert_eq!(latest_sample.reserve_primary_available_percent, Some(90.0));

        wait_for_pool_attempt_row_count(&state.pool, 2).await;
        let summaries = sqlx::query_scalar::<_, Option<String>>(
        "SELECT request_summary_json FROM pool_upstream_request_attempts ORDER BY attempt_index",
    )
    .fetch_all(&state.pool)
    .await
    .expect("load persisted pool attempt summaries");
        assert!(summaries.iter().any(|summary| {
            summary.as_deref().is_some_and(|summary| {
                summary.contains("\"quotaRoute\":\"reserve\"")
                    && summary.contains("\"fallbackReason\":\"normal_quota_exhausted\"")
            })
        }));

        upstream_handle.abort();
        oauth_bridge::reset_test_oauth_codex_upstream_base_url().await;
    });
}
