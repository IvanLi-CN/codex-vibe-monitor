#[derive(sqlx::FromRow)]
struct PersistedLargeTerminalRow {
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    response_raw_path: Option<String>,
    payload: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PersistedTruncatedLargeRow {
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    raw_response: String,
    response_raw_path: Option<String>,
    response_raw_size: Option<i64>,
    response_raw_truncated: i64,
    response_raw_truncated_reason: Option<String>,
    payload: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PersistedCompactRow {
    status: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    total_tokens: Option<i64>,
    raw_response: String,
    response_raw_path: Option<String>,
    response_raw_size: Option<i64>,
    payload: Option<String>,
}

fn assert_truncated_large_stream_row(row: &PersistedTruncatedLargeRow) {
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(42));
    assert_eq!(row.output_tokens, Some(13));
    assert_eq!(row.total_tokens, Some(55));
    assert_eq!(row.raw_response.len(), RAW_RESPONSE_PREVIEW_LIMIT);
    assert_eq!(row.response_raw_truncated, 1);
    assert_eq!(
        row.response_raw_truncated_reason.as_deref(),
        Some("max_bytes_exceeded")
    );
    assert!(
        row.response_raw_size.is_some_and(|size| size > 8 * 1024),
        "response raw size should still reflect the full upstream stream"
    );
    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read truncated response raw");
    assert!(raw_bytes.len() <= 8 * 1024);
    let raw_text = String::from_utf8(raw_bytes).expect("truncated raw response should remain utf8");
    assert!(!raw_text.contains("response.completed"));
    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode persisted payload summary");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["serviceTier"].as_str(), Some("priority"));
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_large_stream_terminal_event_keeps_live_metadata_without_raw_reread()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-terminal-stream-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-terminal-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedLargeTerminalRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedLargeTerminalRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_path,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large terminal stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large terminal stream capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(
        row.response_raw_path.is_some(),
        "full raw response should be stored"
    );
    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read persisted large terminal raw");
    let raw_text =
        String::from_utf8(raw_bytes).expect("large terminal raw response should be utf8");
    assert!(raw_text.contains("response.completed"));
    assert!(raw_text.contains("\"total_tokens\":96"));

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode large terminal payload summary");
    match (row.input_tokens, row.output_tokens, row.total_tokens) {
        (Some(input), Some(output), Some(total)) => {
            assert_eq!(input, 77);
            assert_eq!(output, 19);
            assert_eq!(total, 96);
            assert_eq!(payload["serviceTier"], "priority");
            assert!(payload["usageMissingReason"].is_null());
        }
        (None, None, None) => {
            assert!(
                payload["usageMissingReason"].as_str().is_some(),
                "oversized terminal events may no longer backfill metadata from raw rereads"
            );
        }
        other => panic!("unexpected partial usage extraction state: {other:?}"),
    }
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_oversized_stream_keeps_live_metadata_when_raw_file_is_truncated()
 {
    #[derive(sqlx::FromRow)]
    struct PersistedOversizedRow {
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        total_tokens: Option<i64>,
        response_raw_truncated: i64,
        response_raw_truncated_reason: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-oversized-stream-truncated-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(24 * 1024);
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=oversized-delta-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedOversizedRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedOversizedRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                response_raw_truncated,
                response_raw_truncated_reason,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query oversized stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("oversized stream capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(61));
    assert_eq!(row.output_tokens, Some(17));
    assert_eq!(row.total_tokens, Some(78));
    assert_eq!(row.response_raw_truncated, 1);
    assert_eq!(
        row.response_raw_truncated_reason.as_deref(),
        Some("max_bytes_exceeded")
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode oversized stream payload summary");
    assert_eq!(payload["serviceTier"], "priority");
    assert!(payload["usageMissingReason"].is_null());
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_large_stream_keeps_usage_when_response_raw_is_truncated() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-stream-raw-truncated");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(8 * 1024);
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses?mode=large-stream"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let body_text = String::from_utf8(body.to_vec()).expect("stream body should be utf8");
    assert!(body_text.contains("response.completed"));

    let mut row: Option<PersistedTruncatedLargeRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedTruncatedLargeRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                raw_response,
                response_raw_path,
                response_raw_size,
                response_raw_truncated,
                response_raw_truncated_reason,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query truncated large stream capture row");
        if row
            .as_ref()
            .is_some_and(|record| record.total_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("truncated large stream capture row should exist");
    assert_truncated_large_stream_row(&row);
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_stream_request_json_error_uses_nonstream_parse_fallback() {
    #[derive(sqlx::FromRow)]
    struct PersistedErrorRow {
        status: Option<String>,
        error_message: Option<String>,
        response_raw_truncated: i64,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-stream-json-error");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    config.proxy_raw_max_bytes = Some(128);
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=json-error".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read json error response body");

    let mut row: Option<PersistedErrorRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedErrorRow>(
            r#"
            SELECT
                status,
                error_message,
                response_raw_truncated,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query json error capture row");
        if row
            .as_ref()
            .and_then(|record| record.error_message.as_deref())
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("json error capture row should exist");
    assert_eq!(row.status.as_deref(), Some("http_400"));
    assert_eq!(row.response_raw_truncated, 1);
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|message| message.contains("tail-marker")),
        "stream request should keep the full JSON error message even when raw storage truncates"
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode json error payload");
    assert_eq!(
        payload["upstreamErrorCode"].as_str(),
        Some("invalid_request_error")
    );
    assert!(
        payload["upstreamErrorMessage"]
            .as_str()
            .is_some_and(|message| message.contains("tail-marker")),
        "payload should preserve the full upstream error message"
    );
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "manual RSS soak harness for large proxy response capture"]
pub(crate) async fn proxy_capture_target_large_stream_soak_keeps_rss_within_stable_window() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-stream-soak");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let mut rss_samples = Vec::new();
    for iteration in 0..8_i64 {
        let response = proxy_openai_v1(
            State(state.clone()),
            OriginalUri(
                "/v1/responses?mode=large-stream"
                    .parse()
                    .expect("valid uri"),
            ),
            Method::POST,
            HeaderMap::new(),
            Body::from(r#"{"model":"gpt-5.4","stream":true,"input":"hello"}"#),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        let _ = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read proxy response body");
        wait_for_codex_invocations(&state.pool, iteration + 1).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        if let Some(rss_kib) = current_process_rss_kib() {
            rss_samples.push(rss_kib);
        }
    }

    eprintln!("large-stream-soak rss_kib={rss_samples:?}");
    assert!(
        rss_samples.len() >= 4,
        "RSS harness should collect multiple samples"
    );
    let steady_state = &rss_samples[3..];
    let min_rss = *steady_state
        .iter()
        .min()
        .expect("steady-state RSS should have a minimum");
    let max_rss = *steady_state
        .iter()
        .max()
        .expect("steady-state RSS should have a maximum");
    assert!(
        max_rss.saturating_sub(min_rss) < 128 * 1024,
        "steady-state RSS window too wide: min={min_rss}KiB max={max_rss}KiB"
    );
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

#[tokio::test]
#[ignore = "reverse proxy removed; /v1/* now requires a pool route key"]
pub(crate) async fn proxy_capture_target_large_nonstream_json_skips_bounded_parse_and_keeps_full_raw_file()
 {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let raw_dir = make_temp_test_dir("proxy-large-json-raw");
    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.proxy_raw_dir = raw_dir.clone();
    let state = test_state_from_config(config, true).await;
    reset_proxy_capture_hot_path_raw_fallbacks();

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_large",
        "input": [{ "role": "user", "content": "compact this thread" }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(
            "/v1/responses/compact?mode=large-json"
                .parse()
                .expect("valid compact uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read compact proxy response body");

    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                status,
                input_tokens,
                output_tokens,
                total_tokens,
                raw_response,
                response_raw_path,
                response_raw_size,
                payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query large compact capture row");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("large compact capture row should exist");
    assert_eq!(row.status.as_deref(), Some("success"));
    assert!(row.input_tokens.is_none());
    assert!(row.output_tokens.is_none());
    assert!(row.total_tokens.is_none());
    assert_eq!(row.raw_response.len(), RAW_RESPONSE_PREVIEW_LIMIT);
    assert!(
        row.response_raw_size
            .is_some_and(|size| size > BOUNDED_NON_STREAM_RESPONSE_PARSE_LIMIT_BYTES as i64)
    );

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert!(
        payload["usageMissingReason"]
            .as_str()
            .is_some_and(|reason| reason.contains(PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED))
    );

    let response_raw_path = row
        .response_raw_path
        .as_deref()
        .expect("response raw path should be persisted");
    let raw_bytes =
        read_proxy_raw_bytes(response_raw_path, None).expect("read large compact raw response");
    let raw_text = String::from_utf8(raw_bytes).expect("raw compact response should be utf8");
    assert!(raw_text.contains("\"total_tokens\":300"));
    assert_proxy_capture_hot_path_skips_raw_fallbacks();

    upstream_handle.abort();
    cleanup_temp_test_dir(&raw_dir);
}

use super::*;
