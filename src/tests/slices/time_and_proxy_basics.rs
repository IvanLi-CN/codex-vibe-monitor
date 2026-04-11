use super::*;
use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, KeyInit},
};
use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Query, State},
    http::{HeaderName, HeaderValue, Method, StatusCode, Uri, header as http_header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use brotli::CompressorWriter;
use chrono::Timelike;
use flate2::{
    Compression,
    write::{DeflateEncoder, GzEncoder, ZlibEncoder},
};
use rand::{RngCore, rngs::OsRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::error::{DatabaseError, ErrorKind};
use sqlx::{Connection, SqliteConnection, SqlitePool, sqlite::SqlitePoolOptions};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    convert::Infallible,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, atomic::AtomicUsize},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::sync::{Mutex as AsyncMutex, Notify, Semaphore, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use reqwest::Url;

static APP_CONFIG_ENV_LOCK: once_cell::sync::Lazy<AsyncMutex<()>> =
    once_cell::sync::Lazy::new(|| AsyncMutex::new(()));

struct CurrentDirGuard {
    original: PathBuf,
}

struct EnvVarGuard {
    previous: Vec<(String, Option<OsString>)>,
}

impl CurrentDirGuard {
    fn change_to(path: &Path) -> Self {
        let original = env::current_dir().expect("read current dir");
        env::set_current_dir(path).expect("set current dir");
        Self { original }
    }
}

impl EnvVarGuard {
    fn set(cases: &[(&str, Option<&str>)]) -> Self {
        let previous = cases
            .iter()
            .map(|(name, _)| ((*name).to_string(), env::var_os(name)))
            .collect::<Vec<_>>();

        for (name, value) in cases {
            match value {
                Some(value) => unsafe { env::set_var(name, value) },
                None => unsafe { env::remove_var(name) },
            }
        }

        Self { previous }
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original);
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..).rev() {
            match value {
                Some(value) => unsafe { env::set_var(&name, value) },
                None => unsafe { env::remove_var(&name) },
            }
        }
    }
}

#[test]
fn app_config_from_sources_ignores_proxy_request_concurrency_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_PROXY_REQUEST_CONCURRENCY_LIMIT, Some("7")),
        (ENV_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS, Some("3456")),
    ]);

    let config = AppConfig::from_sources(&CliArgs::default())
        .expect("proxy request concurrency envs should parse");

    assert_eq!(
        config.proxy_request_concurrency_limit,
        DEFAULT_PROXY_REQUEST_CONCURRENCY_LIMIT
    );
    assert_eq!(
        config.proxy_request_concurrency_wait_timeout,
        Duration::from_millis(DEFAULT_PROXY_REQUEST_CONCURRENCY_WAIT_TIMEOUT_MS)
    );
}

#[tokio::test]
async fn acquire_proxy_request_concurrency_permit_tracks_multiple_in_flight_requests() {
    let mut config = test_config();
    config.proxy_request_concurrency_limit = 1;
    config.proxy_request_concurrency_wait_timeout = Duration::from_millis(25);
    let state = test_state_from_config(config, true).await;
    let uri = "/v1/responses".parse::<Uri>().expect("valid proxy uri");

    let permit =
        acquire_proxy_request_concurrency_permit(state.as_ref(), 1002, &Method::POST, &uri)
            .await;
    let permit2 =
        acquire_proxy_request_concurrency_permit(state.as_ref(), 1003, &Method::POST, &uri).await;
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        2
    );

    drop(permit);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );

    drop(permit2);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
}

#[tokio::test]
async fn proxy_openai_v1_invalid_pool_key_bypasses_admission_backpressure() {
    let mut config = test_config();
    config.proxy_request_concurrency_limit = 1;
    config.proxy_request_concurrency_wait_timeout = Duration::from_millis(25);
    let state = test_state_from_config(config, true).await;
    let uri = "/v1/responses".parse::<Uri>().expect("valid proxy uri");

    let permit =
        acquire_proxy_request_concurrency_permit(state.as_ref(), 2001, &Method::POST, &uri).await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri(uri),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer invalid-pool-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(Bytes::from_static(br#"{"model":"gpt-5","input":"hello"}"#)),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read invalid pool key body"),
    )
    .expect("decode invalid pool key payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE)
    );
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        1,
        "invalid pool keys should not consume an admission slot while another request is in flight"
    );

    drop(permit);
}

#[tokio::test]
async fn proxy_openai_v1_models_rejects_non_pool_bearer_key() {
    let state = test_state_with_openai_base(
        Url::parse("https://example.invalid").expect("valid upstream base url"),
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::from_iter([(
            http_header::AUTHORIZATION,
            HeaderValue::from_static("Bearer sk-direct-upstream"),
        )]),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let payload: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read non-pool models error body"),
    )
    .expect("decode non-pool models error payload");
    assert_eq!(
        payload["error"].as_str(),
        Some(PROXY_POOL_ROUTE_KEY_MISSING_OR_INVALID_MESSAGE)
    );
}

#[tokio::test]
async fn proxy_openai_v1_via_pool_keeps_in_flight_tracking_until_downstream_stream_finishes() {
    let app = Router::new().route(
        "/v1/responses",
        post(|| async move {
            let stream = futures_util::stream::once(async {
                Ok::<Bytes, Infallible>(Bytes::from_static(br#"{"phase":"streaming""#))
            })
            .chain(futures_util::stream::once(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok::<Bytes, Infallible>(Bytes::from_static(br#","done":true}"#))
            }));
            Response::builder()
                .status(StatusCode::OK)
                .header(http_header::CONTENT_TYPE, "application/json")
                .body(Body::from_stream(stream))
                .expect("build streaming response")
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind streaming upstream");
    let addr = listener.local_addr().expect("streaming upstream addr");
    let upstream_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("streaming upstream should run");
    });

    let mut config = test_config();
    config.openai_upstream_base_url =
        Url::parse(&format!("http://{addr}")).expect("valid streaming upstream base url");
    config.proxy_request_concurrency_limit = 1;
    config.proxy_request_concurrency_wait_timeout = Duration::from_millis(50);
    let state = test_state_from_config(config, true).await;
    seed_pool_routing_api_key(&state, "pool-stream-slot-key").await;
    insert_test_pool_api_key_account(&state, "Streaming Slot", "route-stream-slot").await;

    let runtime_timeouts = resolve_proxy_request_timeouts(state.as_ref(), true)
        .await
        .expect("resolve pool runtime timeouts");
    let response = proxy_openai_v1_via_pool(
        state.clone(),
        1003,
        &"/v1/responses".parse().expect("valid uri"),
        Method::POST,
        HeaderMap::from_iter([
            (
                http_header::AUTHORIZATION,
                HeaderValue::from_static("Bearer pool-stream-slot-key"),
            ),
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
        ]),
        Body::from(Bytes::from_static(br#"{"model":"gpt-5","input":"hi"}"#)),
        runtime_timeouts,
        None,
    )
    .await
    .expect("streaming via-pool request should succeed");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        1,
        "proxy request should remain in-flight until downstream streaming finishes"
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read streaming via-pool response");
    assert_eq!(
        body,
        Bytes::from_static(br#"{"phase":"streaming","done":true}"#)
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        state
            .proxy_request_in_flight
            .load(std::sync::atomic::Ordering::Acquire),
        0,
        "in-flight tracking should release after downstream streaming completes"
    );

    upstream_handle.abort();
}

#[test]
fn named_range_today_end_respects_dst() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2024, 3, 10, 12, 0, 0)
        .single()
        .expect("valid dt");

    let (start, end) = named_range_bounds("today", now, tz).expect("today bounds");
    // Midnight before DST jump is still PST (-08:00).
    assert_eq!(
        start,
        Utc.with_ymd_and_hms(2024, 3, 10, 8, 0, 0).single().unwrap()
    );
    // Next midnight is PDT (-07:00) after the DST jump.
    assert_eq!(
        end,
        Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
    );
}

#[test]
fn named_range_yesterday_end_respects_dst() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2024, 3, 11, 12, 0, 0)
        .single()
        .expect("valid dt");

    let (start, end) = named_range_bounds("yesterday", now, tz).expect("yesterday bounds");
    // Yesterday start: Sun 2024-03-10 00:00 PST => 08:00Z.
    assert_eq!(
        start,
        Utc.with_ymd_and_hms(2024, 3, 10, 8, 0, 0).single().unwrap()
    );
    // Yesterday end: Mon 2024-03-11 00:00 PDT => 07:00Z after the DST jump.
    assert_eq!(
        end,
        Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
    );
}

#[test]
fn named_range_this_week_end_respects_dst() {
    let tz = chrono_tz::America::Los_Angeles;
    let now = Utc
        .with_ymd_and_hms(2024, 3, 6, 12, 0, 0)
        .single()
        .expect("valid dt");

    let (start, end) = named_range_bounds("thisWeek", now, tz).expect("thisWeek bounds");
    // Start of week: Mon 2024-03-04 00:00 PST => 08:00Z.
    assert_eq!(
        start,
        Utc.with_ymd_and_hms(2024, 3, 4, 8, 0, 0).single().unwrap()
    );
    // End of week: Mon 2024-03-11 00:00 PDT => 07:00Z.
    assert_eq!(
        end,
        Utc.with_ymd_and_hms(2024, 3, 11, 7, 0, 0).single().unwrap()
    );
}

#[test]
fn next_reporting_bucket_epoch_respects_dst_for_multi_hour_buckets() {
    let tz = chrono_tz::America::New_York;
    let timestamp = Utc
        .with_ymd_and_hms(2024, 3, 10, 9, 30, 0)
        .single()
        .expect("valid dt");

    let bucket_start_epoch =
        align_reporting_bucket_epoch(timestamp.timestamp(), 6 * 3_600, tz).expect("align bucket");
    let bucket_end_epoch =
        next_reporting_bucket_epoch(bucket_start_epoch, 6 * 3_600, tz).expect("next bucket");

    let bucket_start_local = Utc
        .timestamp_opt(bucket_start_epoch, 0)
        .single()
        .expect("valid bucket start")
        .with_timezone(&tz);
    let bucket_end_local = Utc
        .timestamp_opt(bucket_end_epoch, 0)
        .single()
        .expect("valid bucket end")
        .with_timezone(&tz);

    assert_eq!(bucket_start_local.hour(), 0);
    assert_eq!(bucket_start_local.minute(), 0);
    assert_eq!(bucket_end_local.hour(), 6);
    assert_eq!(bucket_end_local.minute(), 0);
}

#[test]
fn parse_summary_window_accepts_yesterday_calendar_window() {
    let window = parse_summary_window(
        &SummaryQuery {
            window: Some("yesterday".to_string()),
            limit: None,
            time_zone: None,
        },
        50,
    )
    .expect("parse yesterday summary window");

    match window {
        SummaryWindow::Calendar(value) => assert_eq!(value, "yesterday"),
        other => panic!("expected calendar window, got {other:?}"),
    }
}

#[test]
fn local_naive_to_utc_does_not_fall_back_to_and_utc_on_dst_gap() {
    let tz = chrono_tz::America::Los_Angeles;
    let naive = NaiveDate::from_ymd_opt(2024, 3, 10)
        .unwrap()
        .and_hms_opt(2, 30, 0)
        .unwrap();

    assert!(matches!(tz.from_local_datetime(&naive), LocalResult::None));
    let resolved = local_naive_to_utc(naive, tz);
    assert_ne!(resolved, naive.and_utc());

    let local = resolved.with_timezone(&tz);
    assert_eq!(local.hour(), 3);
    assert_eq!(local.minute(), 0);
    assert_eq!(local.second(), 0);
}

#[test]
fn resolve_invocation_proxy_display_name_prefers_selected_forward_proxy() {
    let selected_proxy = SelectedForwardProxy {
        key: "proxy-a".to_string(),
        source: "manual".to_string(),
        display_name: "Tokyo-Edge-1".to_string(),
        endpoint_url: Some(Url::parse("http://127.0.0.1:7890").expect("valid proxy url")),
        endpoint_url_raw: Some("http://127.0.0.1:7890".to_string()),
    };

    assert_eq!(
        resolve_invocation_proxy_display_name(Some(&selected_proxy)).as_deref(),
        Some("Tokyo-Edge-1")
    );
}

#[test]
fn resolve_invocation_proxy_display_name_returns_none_without_selected_forward_proxy() {
    assert_eq!(resolve_invocation_proxy_display_name(None).as_deref(), None);
}

#[test]
fn normalize_single_proxy_url_supports_scheme_less_host_port() {
    assert_eq!(
        normalize_single_proxy_url("127.0.0.1:7890"),
        Some("http://127.0.0.1:7890".to_string())
    );
    assert_eq!(
        normalize_single_proxy_url("socks5://127.0.0.1:1080"),
        Some("socks5://127.0.0.1:1080".to_string())
    );
    assert_eq!(normalize_single_proxy_url("vmess://example"), None);
}

#[test]
fn normalize_single_proxy_url_supports_xray_share_links() {
    let vmess_payload = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "vmess-node"
    }))
    .expect("serialize vmess payload");
    let vmess_link = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_payload)
    );
    assert!(normalize_single_proxy_url(&vmess_link).is_some());
    assert!(normalize_single_proxy_url("vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless").is_some());
    assert!(normalize_single_proxy_url("trojan://password@trojan.example.com:443?type=ws&path=%2Fws&host=cdn.trojan.example.com").is_some());
    assert!(
        normalize_single_proxy_url("ss://YWVzLTI1Ni1nY206cGFzc0AxMjcuMC4wLjE6ODM4OA==").is_some()
    );
}

#[test]
fn stable_proxy_keys_ignore_share_link_display_name_only_changes() {
    let vmess_payload_a = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "东京节点"
    }))
    .expect("serialize vmess payload a");
    let vmess_payload_b = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "Tokyo Edge"
    }))
    .expect("serialize vmess payload b");
    let vmess_link_a = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_payload_a)
    );
    let vmess_link_b = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_payload_b)
    );
    assert_eq!(
        normalize_single_proxy_key(&vmess_link_a),
        normalize_single_proxy_key(&vmess_link_b)
    );

    assert_eq!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#东京节点"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?type=ws&host=cdn.vless.example.com&path=%2Fws&security=tls#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443#东京节点"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?encryption=none&security=none&type=tcp#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com&sni=edge.vless.example.com&fingerprint=chrome#东京节点"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&net=ws&path=%2Fws&host=cdn.vless.example.com&serverName=edge.vless.example.com&fp=chrome#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?type=ws&path=%2Fws&host=cdn.trojan.example.com#东京节点"
        ),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?host=cdn.trojan.example.com&path=%2Fws&type=ws#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.trojan.example.com&sni=edge.trojan.example.com&fingerprint=chrome#东京节点"
        ),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?security=tls&net=ws&path=%2Fws&host=cdn.trojan.example.com&serverName=edge.trojan.example.com&fp=chrome#Tokyo%20Edge"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key("trojan://password@trojan.example.com:443#东京节点"),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?security=tls&type=tcp#Tokyo%20Edge"
        ),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?type=tcp&headerType=http&host=cdn-a.example.com#节点A"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?type=tcp&headerType=http&host=cdn-b.example.com#节点B"
        ),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?type=kcp&seed=alpha#节点A"
        ),
        normalize_single_proxy_key(
            "trojan://password@trojan.example.com:443?type=kcp&seed=beta#节点B"
        ),
    );
    assert_eq!(
        normalize_single_proxy_key(
            "ss://2022-blake3-aes-128-gcm:%2B%2F%3D@127.0.0.1:8388#东京节点"
        ),
        normalize_single_proxy_key(
            "ss://2022-blake3-aes-128-gcm:%2B%2F%3D@127.0.0.1:8388#Tokyo%20Edge"
        ),
    );

    let stable_http_key =
        normalize_single_proxy_key("http://127.0.0.1:7890").expect("stable http proxy key");
    assert_eq!(
        normalize_bound_proxy_key(&stable_http_key),
        Some(stable_http_key.clone())
    );
    assert_eq!(
        normalize_bound_proxy_key(FORWARD_PROXY_DIRECT_KEY),
        Some(FORWARD_PROXY_DIRECT_KEY.to_string())
    );
}

#[test]
fn stable_proxy_keys_change_when_proxy_identity_changes() {
    let vmess_payload_a = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "type": "none",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "节点A"
    }))
    .expect("serialize vmess payload a");
    let vmess_payload_b = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111",
        "aid": "0",
        "net": "ws",
        "type": "http",
        "host": "cdn.vmess.example.com",
        "path": "/ws",
        "tls": "tls",
        "ps": "节点A"
    }))
    .expect("serialize vmess payload b");
    assert_ne!(
        normalize_single_proxy_key("http://127.0.0.1:7890"),
        normalize_single_proxy_key("http://127.0.0.1:7891"),
    );
    assert_ne!(
        normalize_single_proxy_key(&format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload_a)
        )),
        normalize_single_proxy_key(&format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vmess_payload_b)
        )),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn-a.example.com#节点A"
        ),
        normalize_single_proxy_key(
            "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn-b.example.com#节点A"
        ),
    );
    assert_ne!(
        normalize_single_proxy_key(
            "ss://2022-blake3-aes-128-gcm:%2B%2F%3D@127.0.0.1:8388?plugin=v2ray-plugin%3Btls#节点A"
        ),
        normalize_single_proxy_key(
            "ss://2022-blake3-aes-128-gcm:%2B%2F%3D@127.0.0.1:8388?plugin=obfs-local%3Bobfs%3Dhttp#节点A"
        ),
    );
}

#[test]
fn proxy_display_name_from_url_decodes_non_ascii_fragment() {
    let url = Url::parse(
        "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls#%E4%B8%9C%E4%BA%AC%E8%8A%82%E7%82%B9",
    )
    .expect("valid vless share link");
    assert_eq!(
        proxy_display_name_from_url(&url).as_deref(),
        Some("东京节点")
    );
}

#[tokio::test]
async fn forward_proxy_binding_nodes_restore_display_name_for_missing_bound_keys() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_key = "fpn_deadbeefcafebabe".to_string();
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: proxy_key.clone(),
            display_name: "东京专线 A".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(
                "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.vless.example.com#%E4%B8%9C%E4%BA%AC%E4%B8%93%E7%BA%BF%20A"
                    .to_string(),
            ),
            weight: 0.8,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist forward proxy metadata history");

    let nodes = build_forward_proxy_binding_nodes_response(state.as_ref(), &[proxy_key.clone()])
        .await
        .expect("build binding nodes response");

    assert!(
        nodes
            .iter()
            .any(|node| node.key == FORWARD_PROXY_DIRECT_KEY),
        "direct binding candidate should still be present",
    );
    let missing_node = nodes
        .iter()
        .find(|node| node.key == proxy_key)
        .expect("missing bound node should be present");
    assert_eq!(missing_node.display_name, "东京专线 A");
    assert_eq!(missing_node.source, "missing");
    assert_eq!(missing_node.protocol_label, "UNKNOWN");
    assert!(!missing_node.selectable);
    assert!(!missing_node.penalized);
    assert!(missing_node.last24h.is_empty());
}

#[tokio::test]
async fn load_forward_proxy_runtime_states_maps_legacy_proxy_keys_to_stable_keys() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.vless.example.com#东京专线".to_string();
    let normalized_proxy = normalize_single_proxy_url(&proxy_url).expect("normalize proxy url");
    let stable_proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: normalized_proxy.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(normalized_proxy),
            weight: 0.42,
            success_ema: 0.8,
            latency_ema_ms: Some(123.0),
            consecutive_failures: 1,
        },
    )
    .await
    .expect("persist legacy runtime state");

    let runtime = load_forward_proxy_runtime_states(&state.pool)
        .await
        .expect("load runtime states");
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0].proxy_key, stable_proxy_key);
    assert_eq!(runtime[0].weight, 0.42);
}

#[tokio::test]
async fn load_forward_proxy_runtime_states_maps_legacy_vless_hash_keys_from_current_settings() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京专线".to_string();
    save_forward_proxy_settings(
        &state.pool,
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.clone()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
    )
    .await
    .expect("persist current forward proxy settings");

    let normalized_proxy =
        normalize_share_link_scheme(&proxy_url, "vless").expect("normalize vless proxy url");
    let legacy_proxy_key = {
        let parsed = Url::parse(&normalized_proxy).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let stable_proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    assert_ne!(legacy_proxy_key, stable_proxy_key);
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key,
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            endpoint_url: None,
            weight: 0.42,
            success_ema: 0.8,
            latency_ema_ms: Some(123.0),
            consecutive_failures: 1,
        },
    )
    .await
    .expect("persist legacy hashed runtime state");

    let runtime = load_forward_proxy_runtime_states(&state.pool)
        .await
        .expect("load runtime states");
    assert_eq!(runtime.len(), 1);
    assert_eq!(runtime[0].proxy_key, stable_proxy_key);
    assert_eq!(runtime[0].weight, 0.42);
}

#[tokio::test]
async fn forward_proxy_binding_nodes_reuse_legacy_hourly_stats_for_stable_keys() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.vless.example.com#东京专线".to_string();
    let normalized_proxy = normalize_single_proxy_url(&proxy_url).expect("normalize proxy url");
    let stable_proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: stable_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(normalized_proxy.clone()),
            weight: 0.8,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist legacy runtime state");

    let now_epoch = Utc::now().timestamp();
    let bucket_start_epoch = align_bucket_epoch(now_epoch, 3600, 0);
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
        VALUES (?1, ?2, 5, 4, 1, 4, 480.0, 180.0, datetime('now'))
        "#,
    )
    .bind(&stable_proxy_key)
    .bind(bucket_start_epoch)
    .execute(&state.pool)
    .await
    .expect("insert legacy hourly stats");

    let nodes =
        build_forward_proxy_binding_nodes_response(state.as_ref(), &[stable_proxy_key.clone()])
            .await
            .expect("build binding nodes response");
    let node = nodes
        .into_iter()
        .find(|item| item.key == stable_proxy_key)
        .expect("stable node should be returned");
    let bucket = node
        .last24h
        .into_iter()
        .find(|item| item.success_count == 4 || item.failure_count == 1)
        .expect("matching bucket should exist");
    assert_eq!(bucket.success_count, 4);
    assert_eq!(bucket.failure_count, 1);
}

#[test]
fn forward_proxy_manager_reuses_legacy_vless_hash_runtime_state() {
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京专线";
    let normalized_proxy =
        normalize_share_link_scheme(proxy_url, "vless").expect("normalize vless proxy url");
    let legacy_proxy_key = {
        let parsed = Url::parse(&normalized_proxy).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let stable_proxy_key = normalize_single_proxy_key(proxy_url).expect("normalize proxy key");
    assert_ne!(legacy_proxy_key, stable_proxy_key);

    let manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec![proxy_url.to_string()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            endpoint_url: None,
            weight: 0.37,
            success_ema: 0.9,
            latency_ema_ms: Some(123.0),
            consecutive_failures: 2,
        }],
    );

    let runtime = manager
        .runtime
        .get(&stable_proxy_key)
        .expect("stable runtime should be preserved");
    assert_eq!(runtime.weight, 0.37);
    assert_eq!(
        runtime.endpoint_url.as_deref(),
        Some(normalized_proxy.as_str())
    );
    assert!(!manager.runtime.contains_key(&legacy_proxy_key));
}

#[tokio::test]
async fn forward_proxy_binding_nodes_reuse_legacy_hashed_hourly_stats_from_current_settings() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=tcp#东京专线".to_string();
    let settings = ForwardProxySettings {
        proxy_urls: vec![proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist current forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        for endpoint in &mut manager.endpoints {
            endpoint.endpoint_url = Some(
                Url::parse("socks5://127.0.0.1:11082")
                    .expect("parse synthesized binding endpoint url"),
            );
        }
    }

    let normalized_proxy =
        normalize_share_link_scheme(&proxy_url, "vless").expect("normalize vless proxy url");
    let legacy_proxy_key = {
        let parsed = Url::parse(&normalized_proxy).expect("parse normalized vless url");
        stable_forward_proxy_key(&canonical_share_link_identity(&parsed))
    };
    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&proxy_url, None)
            .expect("binding parts from current proxy url"),
    )[0]
    .clone();

    let now_epoch = Utc::now().timestamp();
    let bucket_start_epoch = align_bucket_epoch(now_epoch, 3600, 0);
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
        VALUES (?1, ?2, 5, 4, 1, 4, 480.0, 180.0, datetime('now'))
        "#,
    )
    .bind(&legacy_proxy_key)
    .bind(bucket_start_epoch)
    .execute(&state.pool)
    .await
    .expect("insert legacy hashed hourly stats");

    let nodes = build_forward_proxy_binding_nodes_response(state.as_ref(), &[])
        .await
        .expect("build binding nodes response");
    let node = nodes
        .into_iter()
        .find(|item| item.key == binding_key)
        .expect("logical binding node should be returned");
    assert!(node.alias_keys.contains(&legacy_proxy_key));
    let bucket = node
        .last24h
        .into_iter()
        .find(|item| item.success_count == 4 || item.failure_count == 1)
        .expect("matching bucket should exist");
    assert_eq!(bucket.success_count, 4);
    assert_eq!(bucket.failure_count, 1);
}

#[tokio::test]
async fn forward_proxy_binding_nodes_map_historical_runtime_keys_to_current_logical_nodes() {
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let current_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Fcurrent&sni=current.example.com#东京专线".to_string();
    let legacy_proxy_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&host=cdn.example.com&path=%2Flegacy&sni=legacy.example.com#东京专线".to_string();
    let settings = ForwardProxySettings {
        proxy_urls: vec![current_proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: false,
    };
    save_forward_proxy_settings(&state.pool, settings.clone())
        .await
        .expect("persist current forward proxy settings");
    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(settings);
        for endpoint in &mut manager.endpoints {
            endpoint.endpoint_url = Some(
                Url::parse("socks5://127.0.0.1:11081")
                    .expect("parse synthesized binding endpoint url"),
            );
        }
    }

    let legacy_proxy_key =
        normalize_single_proxy_key(&legacy_proxy_url).expect("normalize legacy runtime proxy key");
    persist_forward_proxy_runtime_state(
        &state.pool,
        &ForwardProxyRuntimeState {
            proxy_key: legacy_proxy_key.clone(),
            display_name: "东京专线".to_string(),
            source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
            endpoint_url: Some(
                normalize_share_link_scheme(&legacy_proxy_url, "vless")
                    .expect("normalize legacy share link"),
            ),
            weight: 0.55,
            success_ema: 0.78,
            latency_ema_ms: Some(180.0),
            consecutive_failures: 0,
        },
    )
    .await
    .expect("persist legacy runtime state for metadata history");

    let binding_key = forward_proxy_binding_key_candidates(
        &forward_proxy_binding_parts_from_raw(&current_proxy_url, None)
            .expect("binding parts from current proxy url"),
    )[0]
    .clone();
    let nodes =
        build_forward_proxy_binding_nodes_response(state.as_ref(), &[legacy_proxy_key.clone()])
            .await
            .expect("build binding nodes response");
    let current_node = nodes
        .iter()
        .find(|item| item.key == binding_key)
        .expect("current logical node should be returned");
    assert!(current_node.alias_keys.contains(&legacy_proxy_key));
    assert!(
        !nodes
            .iter()
            .any(|item| item.key == legacy_proxy_key && item.source == "missing"),
        "historical runtime key should fold into the current logical node instead of rendering as missing"
    );
}

#[test]
fn parse_proxy_urls_from_subscription_body_supports_xray_links() {
    let vmess_payload = serde_json::to_string(&json!({
        "add": "vmess.example.com",
        "port": "443",
        "id": "11111111-1111-1111-1111-111111111111"
    }))
    .expect("serialize vmess payload");
    let vmess_link = format!(
        "vmess://{}",
        base64::engine::general_purpose::STANDARD.encode(vmess_payload)
    );
    let vless_link =
        "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls";
    let subscription_raw = format!("{vmess_link}\n{vless_link}");
    let encoded = base64::engine::general_purpose::STANDARD.encode(subscription_raw.as_bytes());
    let parsed = parse_proxy_urls_from_subscription_body(&encoded);
    assert!(parsed.iter().any(|item| item.starts_with("vmess://")));
    assert!(parsed.iter().any(|item| item.starts_with("vless://")));
}

#[test]
fn parse_shadowsocks_share_link_decodes_percent_encoded_credentials() {
    let parsed = parse_shadowsocks_share_link(
        "ss://2022-blake3-aes-128-gcm:%2B%2F%3D@127.0.0.1:8388#ss%20node",
    )
    .expect("parse ss2022 link");
    assert_eq!(parsed.method, "2022-blake3-aes-128-gcm");
    assert_eq!(parsed.password, "+/=");
    assert_eq!(parsed.display_name, "ss node");
    assert_eq!(parsed.host, "127.0.0.1");
    assert_eq!(parsed.port, 8388);
}

#[test]
fn parse_shadowsocks_share_link_decodes_percent_encoded_base64_userinfo() {
    let userinfo =
        base64::engine::general_purpose::STANDARD.encode("chacha20-ietf-poly1305:pass+/=");
    let link = format!("ss://{}@127.0.0.1:8388#node", userinfo.replace('=', "%3D"));
    let parsed = parse_shadowsocks_share_link(&link).expect("parse ss base64 userinfo link");
    assert_eq!(parsed.method, "chacha20-ietf-poly1305");
    assert_eq!(parsed.password, "pass+/=");
}

#[test]
fn decode_subscription_payload_supports_base64_blob() {
    let encoded = base64::engine::general_purpose::STANDARD
        .encode("http://127.0.0.1:7890\nsocks5://127.0.0.1:1080");
    let decoded = decode_subscription_payload(&encoded);
    assert!(decoded.contains("http://127.0.0.1:7890"));
    assert!(decoded.contains("socks5://127.0.0.1:1080"));
}

#[test]
fn forward_proxy_validation_timeout_is_split_by_kind() {
    assert_eq!(
        forward_proxy_validation_timeout(ForwardProxyValidationKind::ProxyUrl),
        Duration::from_secs(FORWARD_PROXY_VALIDATION_TIMEOUT_SECS)
    );
    assert_eq!(
        forward_proxy_validation_timeout(ForwardProxyValidationKind::SubscriptionUrl),
        Duration::from_secs(FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_SECS)
    );
}

#[test]
fn remaining_timeout_budget_stops_when_elapsed_reaches_total() {
    let total = Duration::from_secs(60);
    assert_eq!(
        remaining_timeout_budget(total, Duration::from_secs(20)),
        Some(Duration::from_secs(40))
    );
    assert_eq!(
        remaining_timeout_budget(total, Duration::from_secs(60)),
        Some(Duration::ZERO)
    );
    assert_eq!(
        remaining_timeout_budget(total, Duration::from_secs(61)),
        None
    );
}

#[test]
fn timeout_budget_exhausted_treats_zero_budget_as_exhausted() {
    let total = Duration::from_secs(60);
    assert!(!timeout_budget_exhausted(total, Duration::from_secs(59)));
    assert!(timeout_budget_exhausted(total, Duration::from_secs(60)));
    assert!(timeout_budget_exhausted(total, Duration::from_secs(61)));
}

#[test]
fn timeout_seconds_for_message_rounds_subsecond_up_to_one() {
    assert_eq!(timeout_seconds_for_message(Duration::from_millis(1)), 1);
    assert_eq!(timeout_seconds_for_message(Duration::from_secs(5)), 5);
    assert_eq!(timeout_seconds_for_message(Duration::from_millis(5500)), 6);
}

#[test]
fn fallback_proxy_429_retry_delay_uses_exponential_backoff_with_cap() {
    assert_eq!(
        fallback_proxy_429_retry_delay(1),
        Duration::from_millis(500)
    );
    assert_eq!(fallback_proxy_429_retry_delay(2), Duration::from_secs(1));
    assert_eq!(fallback_proxy_429_retry_delay(3), Duration::from_secs(2));
    assert_eq!(fallback_proxy_429_retry_delay(4), Duration::from_secs(4));
    assert_eq!(fallback_proxy_429_retry_delay(5), Duration::from_secs(5));
    assert_eq!(fallback_proxy_429_retry_delay(9), Duration::from_secs(5));
}

#[test]
fn parse_retry_after_delay_supports_seconds_and_http_date() {
    let seconds = HeaderValue::from_static("2");
    assert_eq!(
        parse_retry_after_delay(&seconds),
        Some(Duration::from_secs(2))
    );

    let retry_at = Utc::now() + chrono::Duration::seconds(5);
    let imf_fixdate = retry_at.format("%a, %d %b %Y %H:%M:%S GMT").to_string();
    let rfc850 = retry_at.format("%A, %d-%b-%y %H:%M:%S GMT").to_string();
    let asctime = retry_at.format("%a %b %e %H:%M:%S %Y").to_string();

    for raw in [imf_fixdate, rfc850, asctime] {
        let http_date = HeaderValue::from_str(&raw).expect("valid retry-after date header");
        let parsed =
            parse_retry_after_delay(&http_date).expect("http-date retry-after should parse");
        assert!(parsed >= Duration::from_secs(1));
        assert!(parsed <= Duration::from_secs(5));
    }
}

#[test]
fn parse_retry_after_delay_clamps_large_values() {
    let huge_seconds = HeaderValue::from_static("3600");
    assert_eq!(
        parse_retry_after_delay(&huge_seconds),
        Some(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS
        ))
    );

    let huge_date = (Utc::now() + chrono::Duration::seconds(3600))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let huge_header = HeaderValue::from_str(&huge_date).expect("valid retry-after date header");
    assert_eq!(
        parse_retry_after_delay(&huge_header),
        Some(Duration::from_secs(
            MAX_PROXY_UPSTREAM_429_RETRY_AFTER_DELAY_SECS
        ))
    );
}

#[test]
fn parse_retry_after_delay_rejects_invalid_or_past_values() {
    let invalid = HeaderValue::from_static("not-a-date");
    assert_eq!(parse_retry_after_delay(&invalid), None);

    let blank = HeaderValue::from_static("   ");
    assert_eq!(parse_retry_after_delay(&blank), None);

    let past = (Utc::now() - chrono::Duration::seconds(1))
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let past_header = HeaderValue::from_str(&past).expect("valid past retry-after date header");
    assert_eq!(parse_retry_after_delay(&past_header), None);
}

#[test]
fn validation_probe_reachable_status_accepts_success_auth_and_not_found() {
    for status in [
        StatusCode::OK,
        StatusCode::NO_CONTENT,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::NOT_FOUND,
    ] {
        assert!(
            is_validation_probe_reachable_status(status),
            "status {status} should be reachable"
        );
    }
}

#[test]
fn validation_probe_reachable_status_rejects_non_reachable_codes() {
    for status in [
        StatusCode::PROXY_AUTHENTICATION_REQUIRED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
        StatusCode::BAD_GATEWAY,
        StatusCode::GATEWAY_TIMEOUT,
    ] {
        assert!(
            !is_validation_probe_reachable_status(status),
            "status {status} should not be reachable"
        );
    }
}

#[tokio::test]
async fn validate_proxy_url_candidate_accepts_probe_404() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let response = validate_single_forward_proxy_candidate(state.as_ref(), proxy_url.clone())
        .await
        .expect("404 should be treated as reachable");

    assert!(response.ok);
    assert_eq!(response.message, "proxy validation succeeded");
    assert_eq!(
        response.normalized_value.as_deref(),
        Some(proxy_url.as_str())
    );
    assert_eq!(response.discovered_nodes, Some(1));
    assert!(
        response.latency_ms.unwrap_or_default() >= 0.0,
        "latency should be present"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn validate_proxy_url_candidate_keeps_5xx_as_failure() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let err = validate_single_forward_proxy_candidate(state.as_ref(), proxy_url)
        .await
        .expect_err("5xx should still fail validation");
    let message = format!("{err:#}");
    assert!(
        message.contains("validation probe returned status 500 Internal Server Error"),
        "expected 500 validation probe failure, got: {message}"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn validate_subscription_candidate_accepts_probe_404() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let response = validate_subscription_candidate(state.as_ref(), subscription_url.clone())
        .await
        .expect("404 should be treated as reachable for subscription validation");

    assert!(response.ok);
    assert_eq!(response.message, "subscription validation succeeded");
    assert_eq!(
        response.normalized_value.as_deref(),
        Some(subscription_url.as_str())
    );
    assert_eq!(response.discovered_nodes, Some(1));
    assert!(
        response.latency_ms.unwrap_or_default() >= 0.0,
        "latency should be present"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn validate_subscription_candidate_keeps_5xx_as_failure() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let err = validate_subscription_candidate(state.as_ref(), subscription_url)
        .await
        .expect_err("5xx should still fail subscription validation");
    let message = format!("{err:#}");
    assert!(
        message.contains("subscription proxy probe failed"),
        "expected subscription probe failure context, got: {message}"
    );
    assert!(
        message.contains("validation probe returned status 500 Internal Server Error"),
        "expected 500 validation probe failure, got: {message}"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn validate_single_forward_proxy_candidate_keeps_xray_startup_running_during_shutdown() {
    let temp_root = make_temp_test_dir("xray-validation-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir;
    let state = test_state_from_config(config, false).await;
    state.shutdown.cancel();

    let err = validate_single_forward_proxy_candidate(
        state.as_ref(),
        "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none".to_string(),
    )
    .await
    .expect_err("validation should fail on missing xray binary, not on shutdown cancellation");
    let message = format!("{err:#}");
    assert!(
        message.contains("failed to start xray binary"),
        "expected validation to keep running through xray startup, got: {message}"
    );
    assert!(
        !message.contains("shutdown is in progress"),
        "request-scoped validation should not reuse the global shutdown token: {message}"
    );
}

#[tokio::test]
async fn xray_supervisor_sync_endpoints_keeps_stale_instances_alive_when_shutdown_starts() {
    let temp_root = make_temp_test_dir("xray-sync-stale-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let mut supervisor = XraySupervisor::new("/path/to/non-existent-xray".to_string(), runtime_dir);
    let config_path = temp_root.join("stale-xray.json");
    fs::write(&config_path, "{}").expect("write stale xray config placeholder");
    let child = Command::new("python3")
        .arg("-c")
        .arg(
            "import time
time.sleep(30)
",
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn stale xray child");
    supervisor.instances.insert(
        "stale-xray".to_string(),
        XrayInstance {
            local_proxy_url: Url::parse("socks5://127.0.0.1:1080").expect("valid local proxy url"),
            config_path: config_path.clone(),
            child,
        },
    );
    let shutdown = CancellationToken::new();
    shutdown.cancel();
    let mut endpoints = Vec::new();

    let err = supervisor
        .sync_endpoints(&mut endpoints, &shutdown)
        .await
        .expect_err("shutdown should interrupt xray sync before stale teardown");
    let message = format!("{err:#}");
    assert!(
        message.contains("shutdown is in progress"),
        "expected shutdown-specific sync error, got: {message}"
    );

    let stale_instance = supervisor
        .instances
        .get_mut("stale-xray")
        .expect("stale instance should be preserved until final shutdown drain");
    assert!(
        stale_instance
            .child
            .try_wait()
            .expect("poll stale child after interrupted sync")
            .is_none(),
        "shutdown-interrupted sync should not tear down stale xray instances early"
    );
    let outcome = terminate_child_process(
        &mut stale_instance.child,
        Duration::from_millis(100),
        "stale-test-child",
    )
    .await;
    assert!(
        matches!(
            outcome,
            ChildTerminationOutcome::Graceful | ChildTerminationOutcome::Forced
        ),
        "test cleanup should terminate the preserved stale child"
    );
}

#[tokio::test]
async fn xray_supervisor_ensure_instance_creates_runtime_dir_for_validation_path() {
    let temp_root = make_temp_test_dir("xray-runtime-create");
    let runtime_dir = temp_root.join("nested/runtime");
    let mut supervisor = XraySupervisor::new(
        "/path/to/non-existent-xray".to_string(),
        runtime_dir.clone(),
    );
    let endpoint = ForwardProxyEndpoint {
        key: "xray-validation-test".to_string(),
        source: FORWARD_PROXY_SOURCE_SUBSCRIPTION.to_string(),
        display_name: "xray-validation-test".to_string(),
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
        raw_url: Some(
            "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                .to_string(),
        ),
    };
    let expected_config_path = runtime_dir.join(format!(
        "forward-proxy-{:016x}.json",
        stable_hash_u64(&endpoint.key)
    ));

    let err = supervisor
        .ensure_instance_with_ready_timeout(
            &endpoint,
            Duration::from_millis(50),
            &CancellationToken::new(),
        )
        .await
        .expect_err("non-existent xray binary should fail to start");
    let message = format!("{err:#}");
    assert!(
        runtime_dir.is_dir(),
        "runtime dir should be created before writing xray config"
    );
    assert!(
        message.contains("failed to start xray binary"),
        "expected startup failure after config write path is available, got: {message}"
    );
    assert!(
        !message.contains("failed to write xray config"),
        "runtime dir creation regression: {message}"
    );
    assert!(
        !expected_config_path.exists(),
        "spawn failure should clean temporary xray config file"
    );

    let _ = fs::remove_dir_all(&temp_root);
}

#[cfg(unix)]
#[tokio::test]
async fn probe_forward_proxy_endpoint_returns_none_when_shutdown_interrupts_temporary_xray_startup()
{
    use std::os::unix::fs::PermissionsExt;

    let temp_root = make_temp_test_dir("xray-probe-startup-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let fake_xray = temp_root.join("fake-xray.sh");
    fs::write(
        &fake_xray,
        "#!/bin/sh
sleep 30
",
    )
    .expect("write fake xray binary");
    let mut perms = fs::metadata(&fake_xray)
        .expect("read fake xray metadata")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_xray, perms).expect("chmod fake xray binary");

    let mut config = test_config();
    config.xray_binary = fake_xray.to_string_lossy().to_string();
    config.xray_runtime_dir = runtime_dir.clone();
    let state = test_state_from_config(config, false).await;
    let endpoint = ForwardProxyEndpoint {
        key: "xray-probe-startup-shutdown".to_string(),
        source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
        display_name: "xray-probe-startup-shutdown".to_string(),
        protocol: ForwardProxyProtocol::Vless,
        endpoint_url: None,
        raw_url: Some(
            "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                .to_string(),
        ),
    };
    let shutdown = CancellationToken::new();
    let cancel_shutdown = shutdown.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_shutdown.cancel();
    });

    let result = probe_forward_proxy_endpoint(
        state.as_ref(),
        &endpoint,
        Duration::from_secs(5),
        Some(&shutdown),
    )
    .await
    .expect("shutdown-interrupted xray startup should normalize to a skipped probe");

    cancel_task
        .await
        .expect("shutdown trigger task should finish");
    assert!(
        result.is_none(),
        "shutdown-interrupted temporary xray startup should be treated as a skipped probe"
    );
    assert!(
        state.xray_supervisor.lock().await.instances.is_empty(),
        "temporary xray validation instances should be cleaned up after shutdown"
    );
    if runtime_dir.exists() {
        let mut runtime_entries = fs::read_dir(&runtime_dir)
            .expect("read temporary xray runtime dir after shutdown")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect temporary xray runtime dir entries");
        runtime_entries.retain(|entry| entry.path().is_file());
        assert!(
            runtime_entries.is_empty(),
            "temporary xray shutdown path should not leave runtime files behind"
        );
    }

    cleanup_temp_test_dir(&temp_root);
}

#[tokio::test]
async fn forward_proxy_settings_returns_service_unavailable_when_shutdown_interrupts_xray_sync() {
    let temp_root = make_temp_test_dir("forward-proxy-settings-shutdown");
    let runtime_dir = temp_root.join("runtime");
    let xray_url = "vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless".to_string();
    let normalized_proxy = normalize_single_proxy_url(&xray_url).expect("normalize xray proxy url");
    let proxy_key = normalize_single_proxy_key(&xray_url).expect("normalize xray proxy key");

    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir;
    let state = test_state_from_config(config, false).await;
    state.shutdown.cancel();

    let err = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![xray_url],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect_err("settings update should surface shutdown interruption");

    assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        err.1.contains("interrupted by shutdown"),
        "expected shutdown-specific error, got: {}",
        err.1
    );
    assert!(
        read_forward_proxy_runtime_weight(&state.pool, &proxy_key)
            .await
            .is_none(),
        "shutdown-interrupted route sync should not persist a partial runtime snapshot"
    );
    let saved_settings = load_forward_proxy_settings(&state.pool)
        .await
        .expect("read forward proxy settings after shutdown interruption");
    assert!(
        !saved_settings.proxy_urls.contains(&normalized_proxy),
        "shutdown-interrupted settings update should not persist the new proxy configuration"
    );
    let manager = state.forward_proxy.lock().await;
    assert!(
        !manager.settings.proxy_urls.contains(&normalized_proxy),
        "shutdown-interrupted settings update should roll back the in-memory proxy configuration"
    );
}

#[tokio::test]
async fn forward_proxy_settings_triggers_async_bootstrap_probe_for_added_manual_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;

    let Json(updated) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec![proxy_url],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");
    assert!(updated.proxy_urls.contains(&normalized_proxy));

    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_does_not_probe_when_no_new_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    let request = ForwardProxySettingsUpdateRequest {
        proxy_urls: vec![proxy_url.clone()],
        subscription_urls: Vec::new(),
        subscription_update_interval_secs: 3600,
        insert_direct: true,
    };
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: request.proxy_urls.clone(),
            subscription_urls: request.subscription_urls.clone(),
            subscription_update_interval_secs: request.subscription_update_interval_secs,
            insert_direct: request.insert_direct,
        }),
    )
    .await
    .expect("initial put forward proxy settings should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let first_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;

    let _ = put_forward_proxy_settings(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .expect("repeated put forward proxy settings should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        second_count, first_count,
        "no newly added endpoint should not trigger extra bootstrap probe"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_does_not_reprobe_when_subscription_is_unchanged() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let proxy_key = normalize_single_proxy_key(&proxy_url).expect("normalize test proxy key");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url.clone()],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("initial put forward proxy settings should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &proxy_key, probe_count_before + 1).await;
    let first_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;

    let _ = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("repeated put forward proxy settings should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second_count = count_forward_proxy_probe_attempts(&state.pool, &proxy_key, None).await;
    assert_eq!(
        second_count, first_count,
        "unchanged subscription endpoints should not trigger extra bootstrap probes"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}
