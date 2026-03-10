use super::*;

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{Query, State},
    http::{HeaderValue, Method, StatusCode, Uri, header as http_header},
    response::IntoResponse,
    routing::{any, get, post},
};
use chrono::Timelike;
use flate2::{Compression, write::GzEncoder};
use serde_json::Value;
use sqlx::error::{DatabaseError, ErrorKind};
use sqlx::{Connection, SqliteConnection, SqlitePool};
use std::{
    borrow::Cow,
    collections::HashSet,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex as StdMutex, atomic::AtomicUsize},
    time::Duration,
};
use tokio::net::TcpListener;
use tokio::sync::{Notify, Semaphore, broadcast};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

static APP_CONFIG_ENV_LOCK: once_cell::sync::Lazy<StdMutex<()>> =
    once_cell::sync::Lazy::new(|| StdMutex::new(()));

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

#[tokio::test]
async fn forward_proxy_settings_triggers_async_bootstrap_probe_for_added_manual_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;

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

    wait_for_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, probe_count_before + 1)
        .await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_does_not_probe_when_no_new_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
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
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
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
    wait_for_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, probe_count_before + 1)
        .await;
    let first_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;

    let _ = put_forward_proxy_settings(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .expect("repeated put forward proxy settings should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let second_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    assert_eq!(
        second_count, first_count,
        "no newly added endpoint should not trigger extra bootstrap probe"
    );

    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_does_not_reprobe_when_subscription_is_unchanged() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;

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
    wait_for_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, probe_count_before + 1)
        .await;
    let first_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;

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

    let second_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    assert_eq!(
        second_count, first_count,
        "unchanged subscription endpoints should not trigger extra bootstrap probes"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn refresh_forward_proxy_subscriptions_triggers_bootstrap_probe_for_added_nodes() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    let success_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;

    refresh_forward_proxy_subscriptions(state.clone(), true, None)
        .await
        .expect("refresh subscriptions should succeed");
    wait_for_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, probe_count_before + 1)
        .await;
    let success_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(true)).await;
    assert!(
        success_count > success_count_before,
        "expected at least one successful bootstrap probe attempt from subscription refresh"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn refresh_forward_proxy_subscriptions_skips_probe_for_known_subscription_keys() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::NOT_FOUND).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let (subscription_url, subscription_handle) =
        spawn_test_subscription_source(format!("{proxy_url}\n")).await;
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: Vec::new(),
            subscription_urls: vec![subscription_url],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }
    sync_forward_proxy_routes(state.as_ref())
        .await
        .expect("sync forward proxy routes before subscription refresh");

    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    let known_keys = HashSet::from([normalized_proxy.clone()]);
    refresh_forward_proxy_subscriptions(state.clone(), true, Some(known_keys))
        .await
        .expect("refresh subscriptions should succeed");
    tokio::time::sleep(Duration::from_millis(300)).await;

    let probe_count_after =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    assert_eq!(
        probe_count_after, probe_count_before,
        "known subscription keys should suppress startup-style reprobe"
    );

    subscription_handle.abort();
    proxy_handle.abort();
}

#[tokio::test]
async fn forward_proxy_settings_bootstrap_probe_failure_penalizes_runtime_weight() {
    let (proxy_url, proxy_handle) =
        spawn_test_forward_proxy_status(StatusCode::INTERNAL_SERVER_ERROR).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize test proxy url");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid probe target"),
    )
    .await;
    let probe_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    let failure_count_before =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(false)).await;

    let _ = put_forward_proxy_settings(
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

    wait_for_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, probe_count_before + 1)
        .await;
    let failure_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, Some(false)).await;
    assert!(
        failure_count > failure_count_before,
        "expected at least one failed bootstrap probe attempt"
    );

    let runtime_weight = read_forward_proxy_runtime_weight(&state.pool, &normalized_proxy)
        .await
        .expect("runtime weight should exist");
    assert!(
        runtime_weight < 1.0,
        "expected failed bootstrap probe to penalize runtime weight; got {runtime_weight}"
    );

    proxy_handle.abort();
}

#[test]
fn forward_proxy_manager_keeps_one_positive_weight() {
    let mut manager = ForwardProxyManager::new(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    assert!(manager.runtime.values().any(|entry| entry.weight > 0.0));
}

#[test]
fn forward_proxy_algo_from_str_supports_v1_and_v2() {
    assert_eq!(
        ForwardProxyAlgo::from_str("v1").expect("v1 should parse"),
        ForwardProxyAlgo::V1
    );
    assert_eq!(
        ForwardProxyAlgo::from_str("V2").expect("v2 should parse"),
        ForwardProxyAlgo::V2
    );
    assert!(ForwardProxyAlgo::from_str("unexpected").is_err());
}

#[test]
fn forward_proxy_algo_config_defaults_to_latest_v2() {
    let algo = resolve_forward_proxy_algo_config(None, None).expect("default algo should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
fn forward_proxy_algo_config_accepts_primary_env() {
    let algo =
        resolve_forward_proxy_algo_config(Some("v2"), None).expect("primary env should resolve");
    assert_eq!(algo, ForwardProxyAlgo::V2);
}

#[test]
fn forward_proxy_algo_config_rejects_legacy_env() {
    let err =
        resolve_forward_proxy_algo_config(None, Some("v1")).expect_err("legacy env should fail");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
fn forward_proxy_algo_config_rejects_when_both_env_vars_are_set() {
    let err = resolve_forward_proxy_algo_config(Some("v2"), Some("v1"))
        .expect_err("legacy env should still win as a hard failure");
    assert_eq!(
        err.to_string(),
        "XY_FORWARD_PROXY_ALGO is not supported; rename it to FORWARD_PROXY_ALGO"
    );
}

#[test]
fn forward_proxy_manager_v2_keeps_two_positive_weights() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    for runtime in manager.runtime.values_mut() {
        runtime.weight = -5.0;
    }
    manager.ensure_non_zero_weight();

    let positive_count = manager
        .endpoints
        .iter()
        .filter_map(|endpoint| manager.runtime.get(&endpoint.key))
        .filter(|entry| entry.weight > 0.0)
        .count();
    assert_eq!(positive_count, 2);
}

#[test]
fn forward_proxy_manager_v2_clamps_persisted_runtime_weight_on_startup() {
    let manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        },
        vec![ForwardProxyRuntimeState {
            proxy_key: FORWARD_PROXY_DIRECT_KEY.to_string(),
            display_name: FORWARD_PROXY_DIRECT_LABEL.to_string(),
            source: FORWARD_PROXY_SOURCE_DIRECT.to_string(),
            endpoint_url: None,
            weight: 99.0,
            success_ema: 0.65,
            latency_ema_ms: None,
            consecutive_failures: 0,
        }],
        ForwardProxyAlgo::V2,
    );

    let direct_runtime = manager
        .runtime
        .get(FORWARD_PROXY_DIRECT_KEY)
        .expect("direct runtime should exist");
    assert_eq!(direct_runtime.weight, FORWARD_PROXY_V2_WEIGHT_MAX);
}

#[test]
fn forward_proxy_manager_v2_counts_only_selectable_positive_candidates() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![
                "http://127.0.0.1:7890".to_string(),
                "http://127.0.0.1:7891".to_string(),
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    let selectable_keys = manager
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .collect::<Vec<_>>();
    assert_eq!(selectable_keys.len(), 2);
    let non_selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| !endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("non-selectable endpoint should exist");

    manager
        .runtime
        .get_mut(&selectable_keys[0])
        .expect("selectable runtime should exist")
        .weight = 1.0;
    manager
        .runtime
        .get_mut(&selectable_keys[1])
        .expect("selectable runtime should exist")
        .weight = -5.0;
    manager
        .runtime
        .get_mut(&non_selectable_key)
        .expect("non-selectable runtime should exist")
        .weight = 1.0;

    manager.ensure_non_zero_weight();

    let positive_selectable = selectable_keys
        .iter()
        .filter_map(|key| manager.runtime.get(key))
        .filter(|runtime| runtime.weight > 0.0)
        .count();
    assert_eq!(positive_selectable, 2);
}

#[test]
fn forward_proxy_manager_v2_probe_ignores_non_selectable_penalties() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec![
                "http://127.0.0.1:7890".to_string(),
                "vless://11111111-1111-1111-1111-111111111111@127.0.0.1:443?encryption=none"
                    .to_string(),
            ],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );

    let selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("selectable endpoint should exist");
    let non_selectable_key = manager
        .endpoints
        .iter()
        .find(|endpoint| !endpoint.is_selectable())
        .map(|endpoint| endpoint.key.clone())
        .expect("non-selectable endpoint should exist");

    manager
        .runtime
        .get_mut(&selectable_key)
        .expect("selectable runtime should exist")
        .weight = 1.0;
    manager
        .runtime
        .get_mut(&non_selectable_key)
        .expect("non-selectable runtime should exist")
        .weight = -2.0;

    assert!(!manager.should_probe_penalized_proxy());
    assert!(manager.mark_probe_started().is_none());
}

#[test]
fn forward_proxy_manager_v2_success_with_high_latency_still_gains_weight() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );
    let proxy_key = manager
        .endpoints
        .first()
        .expect("endpoint should exist")
        .key
        .clone();
    let before = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist")
        .weight;

    manager.record_attempt(&proxy_key, true, Some(45_000.0), false);

    let after = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist")
        .weight;
    assert!(after > before, "v2 success should increase weight");
}

#[test]
fn forward_proxy_manager_v2_success_recovers_penalized_proxy() {
    let mut manager = ForwardProxyManager::with_algo(
        ForwardProxySettings {
            proxy_urls: vec!["http://127.0.0.1:7890".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: false,
        },
        vec![],
        ForwardProxyAlgo::V2,
    );
    let proxy_key = manager
        .endpoints
        .first()
        .expect("endpoint should exist")
        .key
        .clone();
    if let Some(runtime) = manager.runtime.get_mut(&proxy_key) {
        runtime.weight = -2.0;
        runtime.consecutive_failures = 5;
        runtime.latency_ema_ms = Some(30_000.0);
    }

    manager.record_attempt(&proxy_key, true, Some(30_000.0), false);

    let runtime = manager
        .runtime
        .get(&proxy_key)
        .expect("runtime should exist");
    assert!(
        runtime.weight >= FORWARD_PROXY_V2_WEIGHT_RECOVERY_FLOOR,
        "successful recovery should restore minimum v2 weight"
    );
    assert_eq!(
        runtime.consecutive_failures, 0,
        "successful attempt should reset failure streak"
    );
}

#[test]
fn classify_invocation_failure_marks_downstream_closed_as_client_abort() {
    let result = classify_invocation_failure(
        Some("http_200"),
        Some("[downstream_closed] downstream closed while streaming upstream response"),
    );
    assert_eq!(result.failure_class, FailureClass::ClientAbort);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("downstream_closed"));
}

#[test]
fn classify_invocation_failure_marks_invalid_key_as_client_failure() {
    let result = classify_invocation_failure(Some("http_401"), Some("Invalid API key format"));
    assert_eq!(result.failure_class, FailureClass::ClientFailure);
    assert!(!result.is_actionable);
    assert_eq!(result.failure_kind.as_deref(), Some("invalid_api_key"));
}

#[test]
fn classify_invocation_failure_marks_upstream_errors_as_service_failure() {
    let result = classify_invocation_failure(
        Some("http_502"),
        Some(
            "[failed_contact_upstream] failed to contact upstream: error sending request for url (https://example.com/v1/responses)",
        ),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("failed_contact_upstream")
    );
}

#[test]
fn resolve_failure_classification_recomputes_actionable_for_missing_legacy_class() {
    let result = resolve_failure_classification(
        Some("http_502"),
        Some("[failed_contact_upstream] upstream unavailable"),
        None,
        None,
        Some(0),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
}

#[test]
fn resolve_failure_classification_overrides_legacy_default_none_for_failures() {
    let result = resolve_failure_classification(
        Some("http_502"),
        Some("[failed_contact_upstream] upstream unavailable"),
        None,
        Some(FailureClass::None.as_str()),
        Some(0),
    );
    assert_eq!(result.failure_class, FailureClass::ServiceFailure);
    assert!(result.is_actionable);
    assert_eq!(
        result.failure_kind.as_deref(),
        Some("failed_contact_upstream")
    );
}

#[test]
fn failure_scope_parse_defaults_to_service() {
    assert_eq!(
        FailureScope::parse(None).expect("default scope"),
        FailureScope::Service
    );
}

#[test]
fn failure_scope_parse_rejects_unknown_value() {
    let err = FailureScope::parse(Some("unexpected")).expect_err("invalid scope should fail");
    assert!(
        err.0
            .to_string()
            .contains("unsupported failure scope: unexpected"),
        "error should mention rejected scope"
    );
}

#[test]
fn app_config_from_sources_ignores_removed_xyai_env_vars() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
    let cases = [
        ("XY_BASE_URL", "not-a-valid-url"),
        ("XY_VIBE_QUOTA_ENDPOINT", "%%%"),
        ("XY_SESSION_COOKIE_NAME", "legacy-cookie"),
        ("XY_SESSION_COOKIE_VALUE", "legacy-secret"),
        ("XY_LEGACY_POLL_ENABLED", "definitely-not-bool"),
        ("XY_SNAPSHOT_MIN_INTERVAL_SECS", "not-a-number"),
    ];
    let previous = cases
        .iter()
        .map(|(name, _)| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    for (name, value) in cases {
        unsafe { env::set_var(name, value) };
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("removed XYAI env vars should be ignored");
    assert_eq!(config.database_path, PathBuf::from("codex_vibe_monitor.db"));
}

#[test]
fn app_config_from_sources_reads_database_path_env() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
    let previous_database = env::var_os(ENV_DATABASE_PATH);
    let previous_legacy = env::var_os(LEGACY_ENV_DATABASE_PATH);

    unsafe {
        env::remove_var(LEGACY_ENV_DATABASE_PATH);
        env::set_var(ENV_DATABASE_PATH, "/tmp/codex-env.sqlite");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    match previous_database {
        Some(value) => unsafe { env::set_var(ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(ENV_DATABASE_PATH) },
    }
    match previous_legacy {
        Some(value) => unsafe { env::set_var(LEGACY_ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(LEGACY_ENV_DATABASE_PATH) },
    }

    let config = result.expect("DATABASE_PATH should configure the database path");
    assert_eq!(config.database_path, PathBuf::from("/tmp/codex-env.sqlite"));
}

#[test]
fn app_config_from_sources_rejects_legacy_database_path_env() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
    let previous_database = env::var_os(ENV_DATABASE_PATH);
    let previous_legacy = env::var_os(LEGACY_ENV_DATABASE_PATH);

    unsafe {
        env::set_var(ENV_DATABASE_PATH, "/tmp/codex-env.sqlite");
        env::set_var(LEGACY_ENV_DATABASE_PATH, "/tmp/codex-legacy.sqlite");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    match previous_database {
        Some(value) => unsafe { env::set_var(ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(ENV_DATABASE_PATH) },
    }
    match previous_legacy {
        Some(value) => unsafe { env::set_var(LEGACY_ENV_DATABASE_PATH, value) },
        None => unsafe { env::remove_var(LEGACY_ENV_DATABASE_PATH) },
    }

    let err = result.expect_err("legacy database env should fail fast");
    assert!(
        err.to_string()
            .contains("XY_DATABASE_PATH is not supported; rename it to DATABASE_PATH"),
        "error should point to the DATABASE_PATH migration"
    );
}

#[test]
fn app_config_from_sources_reads_renamed_public_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
    let mut cases = LEGACY_ENV_RENAMES
        .iter()
        .map(|(legacy, _)| (*legacy, None))
        .collect::<Vec<_>>();
    cases.extend([
        (ENV_POLL_INTERVAL_SECS, Some("11")),
        (ENV_REQUEST_TIMEOUT_SECS, Some("61")),
        (ENV_XRAY_BINARY, Some("/usr/local/bin/xray-custom")),
        (ENV_XRAY_RUNTIME_DIR, Some("/tmp/xray-runtime")),
        (ENV_MAX_PARALLEL_POLLS, Some("7")),
        (ENV_SHARED_CONNECTION_PARALLELISM, Some("3")),
        (ENV_HTTP_BIND, Some("127.0.0.1:39090")),
        (
            ENV_CORS_ALLOWED_ORIGINS,
            Some("https://app.example.com, http://localhost:5173"),
        ),
        (ENV_LIST_LIMIT_MAX, Some("321")),
        (ENV_USER_AGENT, Some("custom-agent/1.0")),
        (ENV_STATIC_DIR, Some("/tmp/static")),
        (ENV_RETENTION_ENABLED, Some("true")),
        (ENV_RETENTION_DRY_RUN, Some("true")),
        (ENV_RETENTION_INTERVAL_SECS, Some("7200")),
        (ENV_RETENTION_BATCH_ROWS, Some("2222")),
        (ENV_ARCHIVE_DIR, Some("/tmp/archive")),
        (ENV_INVOCATION_SUCCESS_FULL_DAYS, Some("31")),
        (ENV_INVOCATION_MAX_DAYS, Some("91")),
        (ENV_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS, Some("32")),
        (ENV_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS, Some("33")),
        (ENV_QUOTA_SNAPSHOT_FULL_DAYS, Some("34")),
        (ENV_FORWARD_PROXY_ALGO, Some("v2")),
    ]);
    let _env = EnvVarGuard::set(&cases);

    let config =
        AppConfig::from_sources(&CliArgs::default()).expect("renamed public envs should parse");

    assert_eq!(config.poll_interval, Duration::from_secs(11));
    assert_eq!(config.request_timeout, Duration::from_secs(61));
    assert_eq!(config.xray_binary, "/usr/local/bin/xray-custom");
    assert_eq!(config.xray_runtime_dir, PathBuf::from("/tmp/xray-runtime"));
    assert_eq!(config.forward_proxy_algo, ForwardProxyAlgo::V2);
    assert_eq!(config.max_parallel_polls, 7);
    assert_eq!(config.shared_connection_parallelism, 3);
    assert_eq!(
        config.http_bind,
        "127.0.0.1:39090".parse().expect("valid socket address")
    );
    assert_eq!(
        config.cors_allowed_origins,
        vec![
            "https://app.example.com".to_string(),
            "http://localhost:5173".to_string(),
        ]
    );
    assert_eq!(config.list_limit_max, 321);
    assert_eq!(config.user_agent, "custom-agent/1.0");
    assert_eq!(config.static_dir, Some(PathBuf::from("/tmp/static")));
    assert!(config.retention_enabled);
    assert!(config.retention_dry_run);
    assert_eq!(config.retention_interval, Duration::from_secs(7200));
    assert_eq!(config.retention_batch_rows, 2222);
    assert_eq!(config.archive_dir, PathBuf::from("/tmp/archive"));
    assert_eq!(config.invocation_success_full_days, 31);
    assert_eq!(config.invocation_max_days, 91);
    assert_eq!(config.forward_proxy_attempts_retention_days, 32);
    assert_eq!(config.stats_source_snapshots_retention_days, 33);
    assert_eq!(config.quota_snapshot_full_days, 34);
}

#[test]
fn app_config_from_sources_rejects_all_legacy_public_env_renames() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");

    for (legacy_name, canonical_name) in LEGACY_ENV_RENAMES {
        let mut cases = LEGACY_ENV_RENAMES
            .iter()
            .map(|(legacy, _)| (*legacy, None))
            .collect::<Vec<_>>();
        let target = cases
            .iter_mut()
            .find(|(name, _)| *name == *legacy_name)
            .expect("legacy env should be present in helper list");
        *target = (*legacy_name, Some("legacy-value"));
        let _env = EnvVarGuard::set(&cases);

        let err = AppConfig::from_sources(&CliArgs::default())
            .expect_err("legacy env should fail fast with a rename hint");
        assert_eq!(
            err.to_string(),
            format!("{legacy_name} is not supported; rename it to {canonical_name}")
        );
    }
}

#[test]
fn app_config_from_sources_uses_proxy_timeout_defaults() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
    let names = [
        "OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS",
    ];
    let previous = names
        .iter()
        .map(|name| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    for name in names {
        unsafe { env::remove_var(name) };
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("proxy timeout defaults should parse");
    assert_eq!(
        config.openai_proxy_handshake_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS)
    );
    assert_eq!(
        config.openai_proxy_compact_handshake_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS)
    );
    assert_eq!(
        config.openai_proxy_request_read_timeout,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS)
    );
}

#[test]
fn app_config_from_sources_reads_proxy_timeout_envs() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("env lock");
    let names = [
        "OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS",
        "OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS",
    ];
    let previous = names
        .iter()
        .map(|name| ((*name).to_string(), env::var_os(name)))
        .collect::<Vec<_>>();

    unsafe {
        env::set_var("OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS", "61");
        env::set_var("OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS", "181");
        env::set_var("OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS", "182");
    }

    let result = AppConfig::from_sources(&CliArgs::default());

    for (name, value) in previous {
        match value {
            Some(value) => unsafe { env::set_var(name, value) },
            None => unsafe { env::remove_var(name) },
        }
    }

    let config = result.expect("proxy timeout envs should parse");
    assert_eq!(
        config.openai_proxy_handshake_timeout,
        Duration::from_secs(61)
    );
    assert_eq!(
        config.openai_proxy_compact_handshake_timeout,
        Duration::from_secs(181)
    );
    assert_eq!(
        config.openai_proxy_request_read_timeout,
        Duration::from_secs(182)
    );
}

fn test_config() -> AppConfig {
    AppConfig {
        openai_upstream_base_url: Url::parse("https://api.openai.com/").expect("valid url"),
        database_path: PathBuf::from(":memory:"),
        poll_interval: Duration::from_secs(10),
        request_timeout: Duration::from_secs(30),
        openai_proxy_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_compact_handshake_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS,
        ),
        openai_proxy_request_read_timeout: Duration::from_secs(
            DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS,
        ),
        openai_proxy_max_request_body_bytes: DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        proxy_enforce_stream_include_usage: DEFAULT_PROXY_ENFORCE_STREAM_INCLUDE_USAGE,
        proxy_usage_backfill_on_startup: DEFAULT_PROXY_USAGE_BACKFILL_ON_STARTUP,
        proxy_raw_max_bytes: DEFAULT_PROXY_RAW_MAX_BYTES,
        proxy_raw_retention: Duration::from_secs(DEFAULT_PROXY_RAW_RETENTION_DAYS * 86_400),
        proxy_raw_dir: PathBuf::from("target/proxy-raw-tests"),
        xray_binary: DEFAULT_XRAY_BINARY.to_string(),
        xray_runtime_dir: PathBuf::from("target/xray-forward-tests"),
        forward_proxy_algo: ForwardProxyAlgo::V1,
        max_parallel_polls: 2,
        shared_connection_parallelism: 1,
        http_bind: "127.0.0.1:0".parse().expect("valid socket address"),
        cors_allowed_origins: Vec::new(),
        list_limit_max: 100,
        user_agent: "codex-test".to_string(),
        static_dir: None,
        retention_enabled: DEFAULT_RETENTION_ENABLED,
        retention_dry_run: DEFAULT_RETENTION_DRY_RUN,
        retention_interval: Duration::from_secs(DEFAULT_RETENTION_INTERVAL_SECS),
        retention_batch_rows: DEFAULT_RETENTION_BATCH_ROWS,
        archive_dir: PathBuf::from("target/archive-tests"),
        invocation_success_full_days: DEFAULT_INVOCATION_SUCCESS_FULL_DAYS,
        invocation_max_days: DEFAULT_INVOCATION_MAX_DAYS,
        forward_proxy_attempts_retention_days: DEFAULT_FORWARD_PROXY_ATTEMPTS_RETENTION_DAYS,
        stats_source_snapshots_retention_days: DEFAULT_STATS_SOURCE_SNAPSHOTS_RETENTION_DAYS,
        quota_snapshot_full_days: DEFAULT_QUOTA_SNAPSHOT_FULL_DAYS,
        crs_stats: None,
    }
}

fn make_temp_test_dir(prefix: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&dir).expect("create temp test dir");
    dir
}

fn set_file_mtime_seconds_ago(path: &Path, seconds: u64) {
    let modified_at = std::time::SystemTime::now() - Duration::from_secs(seconds);
    let modified_at = filetime::FileTime::from_system_time(modified_at);
    filetime::set_file_mtime(path, modified_at).expect("set file mtime");
}

#[test]
fn archive_batch_file_path_resolves_relative_archive_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.archive_dir = PathBuf::from("archives");

    let path = archive_batch_file_path(&config, "codex_invocations", "2026-03")
        .expect("resolve archive batch path");

    assert_eq!(
        path,
        PathBuf::from(
            "/tmp/codex-retention/archives/codex_invocations/2026/codex_invocations-2026-03.sqlite.gz",
        )
    );
}

#[test]
fn resolved_proxy_raw_dir_resolves_relative_dir_from_database_parent() {
    let mut config = test_config();
    config.database_path = PathBuf::from("/tmp/codex-retention/codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    assert_eq!(
        config.resolved_proxy_raw_dir(),
        PathBuf::from("/tmp/codex-retention/proxy_raw_payloads")
    );
}

#[test]
fn store_raw_payload_file_anchors_relative_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("cwd lock");
    let temp_dir = make_temp_test_dir("proxy-raw-store-db-parent");
    let cwd = temp_dir.join("cwd");
    let db_root = temp_dir.join("db-root");
    fs::create_dir_all(&cwd).expect("create cwd dir");
    fs::create_dir_all(&db_root).expect("create db root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let mut config = test_config();
    config.database_path = db_root.join("codex_vibe_monitor.db");
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let meta = store_raw_payload_file(&config, "proxy-test", "request", b"{\"ok\":true}");
    let expected = db_root.join("proxy_raw_payloads/proxy-test-request.bin");

    assert_eq!(
        meta.path.as_deref(),
        Some(expected.to_string_lossy().as_ref())
    );
    assert!(
        expected.exists(),
        "raw payload should be written beside the database"
    );
    assert!(
        !cwd.join("proxy_raw_payloads/proxy-test-request.bin")
            .exists(),
        "raw payload should not follow the current working directory"
    );

    cleanup_temp_test_dir(&temp_dir);
}

#[test]
fn read_proxy_raw_bytes_keeps_current_dir_compat_for_legacy_relative_paths() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("cwd lock");
    let temp_dir = make_temp_test_dir("proxy-raw-read-legacy-cwd");
    let cwd = temp_dir.join("cwd");
    let fallback_root = temp_dir.join("fallback");
    let relative_path = PathBuf::from("proxy_raw_payloads/legacy-request.bin");
    let cwd_path = cwd.join(&relative_path);
    let fallback_path = fallback_root.join(&relative_path);
    fs::create_dir_all(cwd_path.parent().expect("cwd parent")).expect("create cwd raw dir");
    fs::create_dir_all(fallback_path.parent().expect("fallback parent"))
        .expect("create fallback raw dir");
    fs::write(&cwd_path, b"cwd-copy").expect("write cwd raw file");
    fs::write(&fallback_path, b"fallback-copy").expect("write fallback raw file");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd);

    let raw = read_proxy_raw_bytes(
        relative_path.to_str().expect("utf-8 path"),
        Some(&fallback_root),
    )
    .expect("read legacy cwd-relative raw file");

    assert_eq!(raw, b"cwd-copy");
    cleanup_temp_test_dir(&temp_dir);
}

fn sqlite_url_for_path(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy())
}

async fn retention_test_pool_and_config(prefix: &str) -> (SqlitePool, AppConfig, PathBuf) {
    let temp_dir = make_temp_test_dir(prefix);
    let db_path = temp_dir.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create retention sqlite file");
    let db_url = sqlite_url_for_path(&db_path);
    let pool = SqlitePool::connect(&db_url)
        .await
        .expect("connect retention sqlite");
    ensure_schema(&pool).await.expect("ensure retention schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = temp_dir.join("proxy_raw_payloads");
    config.archive_dir = temp_dir.join("archives");
    config.retention_batch_rows = 2;
    fs::create_dir_all(&config.proxy_raw_dir).expect("create retention raw dir");
    fs::create_dir_all(&config.archive_dir).expect("create retention archive dir");
    (pool, config, temp_dir)
}

fn cleanup_temp_test_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn shanghai_local_days_ago(days: i64, hour: u32, minute: u32, second: u32) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(naive)
}

fn utc_naive_from_shanghai_local_days_ago(
    days: i64,
    hour: u32,
    minute: u32,
    second: u32,
) -> String {
    let now_local = Utc::now().with_timezone(&Shanghai);
    let local_naive = (now_local.date_naive() - ChronoDuration::days(days))
        .and_hms_opt(hour, minute, second)
        .expect("valid shanghai local time");
    format_naive(local_naive_to_utc(local_naive, Shanghai).naive_utc())
}

#[allow(clippy::too_many_arguments)]
async fn insert_retention_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    source: &str,
    status: &str,
    payload: Option<&str>,
    raw_response: &str,
    request_raw_path: Option<&Path>,
    response_raw_path: Option<&Path>,
    total_tokens: Option<i64>,
    cost: Option<f64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            input_tokens,
            output_tokens,
            total_tokens,
            cost,
            status,
            payload,
            raw_response,
            request_raw_path,
            request_raw_size,
            response_raw_path,
            response_raw_size,
            raw_expires_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(source)
    .bind(Some("gpt-5.2-codex"))
    .bind(Some(12_i64))
    .bind(Some(3_i64))
    .bind(total_tokens)
    .bind(cost)
    .bind(status)
    .bind(payload)
    .bind(raw_response)
    .bind(request_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(
        request_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .bind(response_raw_path.map(|path| path.to_string_lossy().to_string()))
    .bind(
        response_raw_path
            .and_then(|path| fs::metadata(path).ok())
            .map(|meta| meta.len() as i64),
    )
    .bind(Some("2099-01-01 00:00:00"))
    .execute(pool)
    .await
    .expect("insert retention invocation");
}

async fn insert_stats_source_snapshot_row(pool: &SqlitePool, captured_at: &str, stats_date: &str) {
    sqlx::query(
        r#"
        INSERT INTO stats_source_snapshots (
            source,
            period,
            stats_date,
            model,
            requests,
            input_tokens,
            output_tokens,
            cache_create_tokens,
            cache_read_tokens,
            all_tokens,
            cost_input,
            cost_output,
            cost_cache_write,
            cost_cache_read,
            cost_total,
            raw_response,
            captured_at,
            captured_at_epoch
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
        "#,
    )
    .bind(SOURCE_CRS)
    .bind("daily")
    .bind(stats_date)
    .bind(Some("gpt-5.2"))
    .bind(4_i64)
    .bind(10_i64)
    .bind(6_i64)
    .bind(0_i64)
    .bind(0_i64)
    .bind(16_i64)
    .bind(0.1_f64)
    .bind(0.2_f64)
    .bind(0.0_f64)
    .bind(0.0_f64)
    .bind(0.3_f64)
    .bind("{}")
    .bind(captured_at)
    .bind(
        parse_utc_naive(captured_at)
            .expect("valid utc naive")
            .and_utc()
            .timestamp(),
    )
    .execute(pool)
    .await
    .expect("insert stats source snapshot row");
}

#[derive(Debug)]
struct FakeSqliteCodeDatabaseError {
    message: &'static str,
    code: &'static str,
}

impl std::fmt::Display for FakeSqliteCodeDatabaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FakeSqliteCodeDatabaseError {}

impl DatabaseError for FakeSqliteCodeDatabaseError {
    fn message(&self) -> &str {
        self.message
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.code))
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

fn write_backfill_response_payload(path: &Path) {
    write_backfill_response_payload_with_service_tier(path, None);
}

fn write_backfill_response_payload_with_service_tier(path: &Path, service_tier: Option<&str>) {
    let mut response = json!({
        "type": "response.completed",
        "response": {
            "usage": {
                "input_tokens": 88,
                "output_tokens": 22,
                "total_tokens": 110,
                "input_tokens_details": { "cached_tokens": 9 },
                "output_tokens_details": { "reasoning_tokens": 3 }
            }
        }
    });
    if let Some(service_tier) = service_tier {
        response["response"]["service_tier"] = Value::String(service_tier.to_string());
    }
    let raw = [
        "event: response.completed".to_string(),
        format!("data: {response}"),
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(path, compressed).expect("write response payload");
}

fn write_backfill_request_payload(path: &Path, prompt_cache_key: Option<&str>) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
}

fn write_backfill_request_payload_with_requested_service_tier(
    path: &Path,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(path, None, None, requested_service_tier, target);
}

fn write_backfill_request_payload_with_reasoning(
    path: &Path,
    prompt_cache_key: Option<&str>,
    reasoning_effort: Option<&str>,
    target: ProxyCaptureTarget,
) {
    write_backfill_request_payload_with_fields(
        path,
        prompt_cache_key,
        reasoning_effort,
        None,
        target,
    );
}

fn write_backfill_request_payload_with_fields(
    path: &Path,
    prompt_cache_key: Option<&str>,
    reasoning_effort: Option<&str>,
    requested_service_tier: Option<&str>,
    target: ProxyCaptureTarget,
) {
    let payload = match target {
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact => {
            let mut payload = json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "metadata": {},
            });
            if let Some(key) = prompt_cache_key {
                payload["metadata"]["prompt_cache_key"] = Value::String(key.to_string());
            }
            if let Some(effort) = reasoning_effort {
                payload["reasoning"] = json!({ "effort": effort });
            }
            if let Some(service_tier) = requested_service_tier {
                payload["service_tier"] = Value::String(service_tier.to_string());
            }
            payload
        }
        ProxyCaptureTarget::ChatCompletions => {
            let mut payload = json!({
                "model": "gpt-5.3-codex",
                "stream": true,
                "messages": [{"role": "user", "content": "hello"}],
            });
            if let Some(key) = prompt_cache_key {
                payload["metadata"] = json!({ "prompt_cache_key": key });
            }
            if let Some(effort) = reasoning_effort {
                payload["reasoning_effort"] = Value::String(effort.to_string());
            }
            if let Some(service_tier) = requested_service_tier {
                payload["serviceTier"] = Value::String(service_tier.to_string());
            }
            payload
        }
    };
    let encoded = serde_json::to_vec(&payload).expect("serialize request payload");
    fs::write(path, encoded).expect("write request payload");
}

async fn insert_proxy_backfill_row(pool: &SqlitePool, invoke_id: &str, response_path: &Path) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requestModel\":null,\"responseModel\":null,\"usageMissingReason\":null,\"requestParseError\":null}",
    )
    .bind("{}")
    .bind(response_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert proxy row");
}

async fn insert_proxy_cost_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    model: Option<&str>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, model, input_tokens, output_tokens, total_tokens, cost, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(model)
    .bind(input_tokens)
    .bind(output_tokens)
    .bind(match (input_tokens, output_tokens) {
        (Some(input), Some(output)) => Some(input + output),
        _ => None,
    })
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert proxy cost row");
}

async fn insert_proxy_prompt_cache_backfill_row(
    pool: &SqlitePool,
    invoke_id: &str,
    request_path: &Path,
    payload: &str,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(invoke_id)
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(payload)
    .bind("{}")
    .bind(request_path.to_string_lossy().to_string())
    .execute(pool)
    .await
    .expect("insert proxy prompt cache key row");
}

async fn test_state_with_openai_base(openai_base: Url) -> Arc<AppState> {
    test_state_with_openai_base_and_body_limit(
        openai_base,
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
    )
    .await
}

async fn test_state_with_openai_base_and_body_limit(
    openai_base: Url,
    body_limit: usize,
) -> Arc<AppState> {
    test_state_with_openai_base_body_limit_and_read_timeout(
        openai_base,
        body_limit,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await
}

async fn test_state_with_openai_base_body_limit_and_read_timeout(
    openai_base: Url,
    body_limit: usize,
    request_read_timeout: Duration,
) -> Arc<AppState> {
    test_state_with_openai_base_and_proxy_timeouts(
        openai_base,
        body_limit,
        Duration::from_secs(DEFAULT_OPENAI_PROXY_HANDSHAKE_TIMEOUT_SECS),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_COMPACT_HANDSHAKE_TIMEOUT_SECS),
        request_read_timeout,
    )
    .await
}

async fn test_state_with_openai_base_and_proxy_timeouts(
    openai_base: Url,
    body_limit: usize,
    handshake_timeout: Duration,
    compact_handshake_timeout: Duration,
    request_read_timeout: Duration,
) -> Arc<AppState> {
    let mut config = test_config();
    config.openai_upstream_base_url = openai_base;
    config.openai_proxy_max_request_body_bytes = body_limit;
    config.openai_proxy_handshake_timeout = handshake_timeout;
    config.openai_proxy_compact_handshake_timeout = compact_handshake_timeout;
    config.openai_proxy_request_read_timeout = request_read_timeout;
    test_state_from_config(config, true).await
}

async fn test_state_from_config(config: AppConfig, startup_ready: bool) -> Arc<AppState> {
    let db_id = NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let db_url = format!("sqlite:file:codex-vibe-monitor-test-{db_id}?mode=memory&cache=shared");
    let pool = SqlitePool::connect(&db_url)
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let pricing_catalog = load_pricing_catalog(&pool)
        .await
        .expect("pricing catalog should initialize");

    Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(startup_ready)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(pricing_catalog)),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    })
}

fn test_stage_timings() -> StageTimings {
    StageTimings {
        t_total_ms: 0.0,
        t_req_read_ms: 0.0,
        t_req_parse_ms: 0.0,
        t_upstream_connect_ms: 0.0,
        t_upstream_ttfb_ms: 0.0,
        t_upstream_stream_ms: 0.0,
        t_resp_parse_ms: 0.0,
        t_persist_ms: 0.0,
    }
}

fn test_proxy_capture_record(invoke_id: &str, occurred_at: &str) -> ProxyCaptureRecord {
    ProxyCaptureRecord {
        invoke_id: invoke_id.to_string(),
        occurred_at: occurred_at.to_string(),
        model: Some("gpt-5.2-codex".to_string()),
        usage: ParsedUsage {
            input_tokens: Some(12),
            output_tokens: Some(3),
            cache_input_tokens: Some(2),
            reasoning_tokens: Some(0),
            total_tokens: Some(15),
        },
        cost: Some(0.0123),
        cost_estimated: true,
        price_version: Some("unit-test".to_string()),
        status: "success".to_string(),
        error_message: None,
        payload: Some(
            "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":false,\"requesterIp\":\"198.51.100.77\",\"promptCacheKey\":\"pck-broadcast-1\",\"requestedServiceTier\":\"priority\",\"reasoningEffort\":\"high\"}"
                .to_string(),
        ),
        raw_response: "{}".to_string(),
        req_raw: RawPayloadMeta::default(),
        resp_raw: RawPayloadMeta::default(),
        raw_expires_at: None,
        timings: test_stage_timings(),
    }
}

async fn seed_quota_snapshot(pool: &SqlitePool, captured_at: &str) {
    sqlx::query(
        r#"
        INSERT INTO codex_quota_snapshots (
            captured_at,
            amount_limit,
            used_amount,
            remaining_amount,
            period,
            period_reset_time,
            expire_time,
            is_active,
            total_cost,
            total_requests,
            total_tokens,
            last_request_time,
            billing_type,
            remaining_count,
            used_count,
            sub_type_name
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        "#,
    )
    .bind(captured_at)
    .bind(Some(100.0))
    .bind(Some(10.0))
    .bind(Some(90.0))
    .bind(Some("monthly"))
    .bind(Some("2026-03-01 00:00:00"))
    .bind(None::<String>)
    .bind(1_i64)
    .bind(10.0)
    .bind(9_i64)
    .bind(150_i64)
    .bind(Some(captured_at))
    .bind(Some("prepaid"))
    .bind(Some(91_i64))
    .bind(Some(9_i64))
    .bind(Some("unit"))
    .execute(pool)
    .await
    .expect("seed quota snapshot");
}

async fn seed_forward_proxy_attempt_at(
    pool: &SqlitePool,
    proxy_key: &str,
    occurred_at: DateTime<Utc>,
    is_success: bool,
) {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_attempts (
            proxy_key,
            occurred_at,
            is_success,
            latency_ms,
            failure_kind,
            is_probe
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind(proxy_key)
    .bind(occurred_at.format("%Y-%m-%d %H:%M:%S").to_string())
    .bind(is_success as i64)
    .bind(if is_success { Some(120.0) } else { None })
    .bind(if is_success {
        None::<String>
    } else {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR.to_string())
    })
    .bind(0_i64)
    .execute(pool)
    .await
    .expect("seed forward proxy attempt");
}

#[allow(clippy::too_many_arguments)]
async fn seed_forward_proxy_weight_bucket_at(
    pool: &SqlitePool,
    proxy_key: &str,
    bucket_start_epoch: i64,
    sample_count: i64,
    min_weight: f64,
    max_weight: f64,
    avg_weight: f64,
    last_weight: f64,
) {
    sqlx::query(
        r#"
        INSERT INTO forward_proxy_weight_hourly (
            proxy_key,
            bucket_start_epoch,
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
        ON CONFLICT(proxy_key, bucket_start_epoch) DO UPDATE SET
            sample_count = excluded.sample_count,
            min_weight = excluded.min_weight,
            max_weight = excluded.max_weight,
            avg_weight = excluded.avg_weight,
            last_weight = excluded.last_weight,
            updated_at = datetime('now')
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .bind(sample_count)
    .bind(min_weight)
    .bind(max_weight)
    .bind(avg_weight)
    .bind(last_weight)
    .execute(pool)
    .await
    .expect("seed forward proxy weight bucket");
}

async fn drain_broadcast_messages(rx: &mut broadcast::Receiver<BroadcastPayload>) {
    loop {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => break,
        }
    }
}

async fn spawn_test_forward_proxy_status(status: StatusCode) -> (String, JoinHandle<()>) {
    let app = Router::new().fallback(any(move || async move {
        (
            status,
            Json(json!({
                "status": status.as_u16(),
            })),
        )
    }));

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind forward proxy status test server");
    let addr = listener
        .local_addr()
        .expect("forward proxy status test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("forward proxy status test server should run");
    });

    (format!("http://{addr}"), handle)
}

async fn spawn_test_subscription_source(body: String) -> (String, JoinHandle<()>) {
    let body = Arc::new(body);
    let app = Router::new().route(
        "/subscription",
        get({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move { (StatusCode::OK, body.as_str().to_string()) }
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind subscription source test server");
    let addr = listener
        .local_addr()
        .expect("subscription source test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("subscription source test server should run");
    });

    (format!("http://{addr}/subscription"), handle)
}

async fn count_forward_proxy_probe_attempts(
    pool: &SqlitePool,
    proxy_key: &str,
    success: Option<bool>,
) -> i64 {
    let query = match success {
        Some(true) => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success != 0"
        }
        Some(false) => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0 AND is_success = 0"
        }
        None => {
            "SELECT COUNT(*) FROM forward_proxy_attempts WHERE proxy_key = ?1 AND is_probe != 0"
        }
    };
    sqlx::query_scalar(query)
        .bind(proxy_key)
        .fetch_one(pool)
        .await
        .expect("count forward proxy probe attempts")
}

async fn wait_for_forward_proxy_probe_attempts(
    pool: &SqlitePool,
    proxy_key: &str,
    expected_min_count: i64,
) {
    let started = Instant::now();
    loop {
        let count = count_forward_proxy_probe_attempts(pool, proxy_key, None).await;
        if count >= expected_min_count {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timed out waiting forward proxy probe attempts for {proxy_key}; expected at least {expected_min_count}, got {count}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn read_forward_proxy_runtime_weight(pool: &SqlitePool, proxy_key: &str) -> Option<f64> {
    sqlx::query_scalar::<_, f64>("SELECT weight FROM forward_proxy_runtime WHERE proxy_key = ?1")
        .bind(proxy_key)
        .fetch_optional(pool)
        .await
        .expect("read forward proxy runtime weight")
}

async fn test_upstream_echo(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let auth = headers
        .get(http_header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let host_header = headers
        .get(http_header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let connection_seen = headers.contains_key(http_header::CONNECTION);
    let x_foo_seen = headers.contains_key(http_header::HeaderName::from_static("x-foo"));
    let x_forwarded_for_seen =
        headers.contains_key(http_header::HeaderName::from_static("x-forwarded-for"));
    let forwarded_seen = headers.contains_key(http_header::HeaderName::from_static("forwarded"));
    let via_seen = headers.contains_key(http_header::HeaderName::from_static("via"));
    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        http_header::HeaderName::from_static("x-upstream"),
        HeaderValue::from_static("ok"),
    );
    response_headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("x-upstream-hop"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("x-upstream-hop"),
        HeaderValue::from_static("should-be-filtered"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("via"),
        HeaderValue::from_static("1.1 upstream-proxy"),
    );
    response_headers.insert(
        http_header::HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=192.0.2.1;proto=https;host=api.example.com"),
    );

    (
        StatusCode::CREATED,
        response_headers,
        Json(json!({
            "method": method.as_str(),
            "path": uri.path(),
            "query": uri.query().unwrap_or_default(),
            "authorization": auth,
            "hostHeader": host_header,
            "connectionSeen": connection_seen,
            "xFooSeen": x_foo_seen,
            "xForwardedForSeen": x_forwarded_for_seen,
            "forwardedSeen": forwarded_seen,
            "viaSeen": via_seen,
            "body": body,
        })),
    )
}

async fn test_upstream_stream() -> impl IntoResponse {
    let chunks = stream::iter(vec![
        Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")),
        Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")),
    ]);
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_stream_first_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-first-chunk-error")),
                    1,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_stream_mid_error() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((Ok::<Bytes, io::Error>(Bytes::from_static(b"chunk-a")), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                Some((
                    Err::<Bytes, io::Error>(io::Error::other("upstream-mid-stream-error")),
                    2,
                ))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_slow_stream() -> impl IntoResponse {
    let chunks = stream::unfold(0usize, |state| async move {
        match state {
            0 => Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-a")), 1)),
            1 => {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"chunk-b")), 2))
            }
            _ => None,
        }
    });
    (
        StatusCode::OK,
        [(
            http_header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )],
        Body::from_stream(chunks),
    )
}

async fn test_upstream_hang() -> impl IntoResponse {
    tokio::time::sleep(Duration::from_secs(2)).await;
    StatusCode::NO_CONTENT
}

async fn test_upstream_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("/v1/echo?from=redirect"),
        )],
        Body::empty(),
    )
}

async fn test_upstream_external_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("https://example.org/outside"),
        )],
        Body::empty(),
    )
}

async fn test_upstream_chat_external_redirect() -> impl IntoResponse {
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(
            http_header::LOCATION,
            HeaderValue::from_static("https://example.org/outside"),
        )],
        Body::empty(),
    )
}

async fn test_upstream_responses_gzip_stream() -> impl IntoResponse {
    let payload = [
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}\n\n",
        "event: response.completed\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}\n\n",
    ]
    .concat();

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(payload.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    (
        StatusCode::OK,
        [
            (
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("text/event-stream"),
            ),
            (
                http_header::CONTENT_ENCODING,
                HeaderValue::from_static("gzip"),
            ),
        ],
        Body::from(compressed),
    )
}

async fn test_upstream_responses(uri: Uri) -> Response {
    if uri.query().is_some_and(|query| query.contains("mode=gzip")) {
        test_upstream_responses_gzip_stream().await.into_response()
    } else if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
        (
            StatusCode::OK,
            Json(json!({
                "id": "resp_delayed_test",
                "object": "response",
                "model": "gpt-5.3-codex",
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 3,
                    "total_tokens": 15
                }
            })),
        )
            .into_response()
    } else {
        test_upstream_stream_mid_error().await.into_response()
    }
}

async fn test_upstream_responses_compact(uri: Uri) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_compact_test",
            "object": "response.compaction",
            "output": [
                {
                    "id": "cmp_001",
                    "type": "compaction",
                    "encrypted_content": "encrypted-summary"
                }
            ],
            "usage": {
                "input_tokens": 139,
                "input_tokens_details": {
                    "cached_tokens": 11
                },
                "output_tokens": 438,
                "output_tokens_details": {
                    "reasoning_tokens": 64
                },
                "total_tokens": 577
            }
        })),
    )
}

async fn test_upstream_models(uri: Uri) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=error"))
    {
        return (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": "upstream model list unavailable"
            })),
        )
            .into_response();
    }

    if uri
        .query()
        .is_some_and(|query| query.contains("mode=slow-body"))
    {
        let chunked = stream::unfold(0u8, |state| async move {
            match state {
                0 => Some((
                    Ok::<Bytes, Infallible>(Bytes::from_static(br#"{"object":"list","data":["#)),
                    1,
                )),
                1 => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    Some((
                        Ok::<Bytes, Infallible>(Bytes::from_static(
                            br#"{"id":"slow-model","object":"model"}]}"#,
                        )),
                        2,
                    ))
                }
                _ => None,
            }
        });
        return (
            StatusCode::OK,
            [(
                http_header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            Body::from_stream(chunked),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(json!({
            "object": "list",
            "data": [
                {
                    "id": "upstream-model-a",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345678
                },
                {
                    "id": "gpt-5.2-codex",
                    "object": "model",
                    "owned_by": "upstream",
                    "created": 1712345679
                }
            ]
        })),
    )
        .into_response()
}

async fn spawn_test_upstream() -> (String, JoinHandle<()>) {
    let app = Router::new()
        .route("/v1/echo", any(test_upstream_echo))
        .route("/v1/stream", any(test_upstream_stream))
        .route(
            "/v1/stream-first-error",
            any(test_upstream_stream_first_error),
        )
        .route("/v1/stream-mid-error", any(test_upstream_stream_mid_error))
        .route("/v1/slow-stream", any(test_upstream_slow_stream))
        .route("/v1/hang", any(test_upstream_hang))
        .route("/v1/models", get(test_upstream_models))
        .route("/v1/redirect", any(test_upstream_redirect))
        .route(
            "/v1/redirect-external",
            any(test_upstream_external_redirect),
        )
        .route(
            "/v1/chat/completions",
            any(test_upstream_chat_external_redirect),
        )
        .route("/v1/responses", any(test_upstream_responses))
        .route(
            "/v1/responses/compact",
            post(test_upstream_responses_compact),
        );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream test server");
    let addr = listener.local_addr().expect("upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("upstream test server should run");
    });

    (format!("http://{addr}/"), handle)
}

async fn test_upstream_capture_target_echo(
    State(captured): State<Arc<Mutex<Vec<Value>>>>,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
    captured.lock().await.push(payload.clone());

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_test",
            "object": "response",
            "model": "gpt-5.3-codex",
            "service_tier": "priority",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 3,
                "total_tokens": 15
            },
            "received": payload,
        })),
    )
}

async fn test_upstream_capture_target_compact_echo(
    State(captured): State<Arc<Mutex<Vec<Value>>>>,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    if uri
        .query()
        .is_some_and(|query| query.contains("mode=delay"))
    {
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let payload: Value = serde_json::from_slice(&body).expect("decode upstream captured body");
    captured.lock().await.push(payload.clone());

    (
        StatusCode::OK,
        Json(json!({
            "id": "resp_compact_test",
            "object": "response.compaction",
            "output": [
                {
                    "id": "cmp_001",
                    "type": "compaction",
                    "encrypted_content": "encrypted-summary"
                }
            ],
            "usage": {
                "input_tokens": 139,
                "input_tokens_details": {
                    "cached_tokens": 11
                },
                "output_tokens": 438,
                "output_tokens_details": {
                    "reasoning_tokens": 64
                },
                "total_tokens": 577
            },
            "received": payload,
        })),
    )
}

async fn spawn_capture_target_body_upstream() -> (String, Arc<Mutex<Vec<Value>>>, JoinHandle<()>) {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(test_upstream_capture_target_echo),
        )
        .route("/v1/responses", post(test_upstream_capture_target_echo))
        .route(
            "/v1/responses/compact",
            post(test_upstream_capture_target_compact_echo),
        )
        .with_state(captured.clone());

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind capture-target upstream test server");
    let addr = listener
        .local_addr()
        .expect("capture-target upstream local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("capture-target upstream test server should run");
    });

    (format!("http://{addr}/"), captured, handle)
}

fn extract_model_ids(payload: &Value) -> Vec<String> {
    payload
        .get("data")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("id").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect()
}

#[test]
fn build_proxy_upstream_url_preserves_path_prefix_and_query() {
    let base = Url::parse("https://proxy.example.com/gateway").expect("valid base");
    let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
    let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
    assert_eq!(
        target.as_str(),
        "https://proxy.example.com/gateway/v1/models?limit=10"
    );
}

#[test]
fn build_proxy_upstream_url_supports_ipv6_literal_base() {
    let base = Url::parse("http://[::1]:8080/gateway/").expect("valid ipv6 base");
    let uri: Uri = "/v1/models?limit=10".parse().expect("valid uri");
    let target = build_proxy_upstream_url(&base, &uri).expect("url should build");
    assert_eq!(
        target.as_str(),
        "http://[::1]:8080/gateway/v1/models?limit=10"
    );
}

#[test]
fn path_has_forbidden_dot_segment_detects_plain_and_encoded_variants() {
    assert!(path_has_forbidden_dot_segment("/v1/../models"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e/models"));
    assert!(path_has_forbidden_dot_segment("/v1/.%2E/models"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%2fadmin"));
    assert!(path_has_forbidden_dot_segment("/v1/%2e%2e%5cadmin"));
    assert!(path_has_forbidden_dot_segment("/v1/%252e%252e%252fadmin"));
    assert!(!path_has_forbidden_dot_segment("/v1/%2efoo/models"));
    assert!(!path_has_forbidden_dot_segment("/v1/models"));
}

#[test]
fn build_proxy_upstream_url_rejects_dot_segment_paths() {
    let base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let uri: Uri = "/v1/%2e%2e%2fadmin?scope=test"
        .parse()
        .expect("valid uri with dot segments");
    let err = build_proxy_upstream_url(&base, &uri).expect_err("dot segments should fail");
    assert!(
        err.to_string().contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED),
        "error should indicate forbidden dot segments: {err}"
    );
}

#[test]
fn has_invalid_percent_encoding_detects_malformed_sequences() {
    assert!(has_invalid_percent_encoding("/v1/%zz/models"));
    assert!(has_invalid_percent_encoding("/v1/%/models"));
    assert!(has_invalid_percent_encoding("/v1/%2/models"));
    assert!(!has_invalid_percent_encoding("/v1/%2F/models"));
    assert!(!has_invalid_percent_encoding("/v1/models"));
}

#[test]
fn should_proxy_header_filters_hop_by_hop_headers() {
    assert!(should_proxy_header(&http_header::AUTHORIZATION));
    assert!(should_proxy_header(&http_header::CONTENT_LENGTH));
    assert!(!should_proxy_header(&http_header::HOST));
    assert!(!should_proxy_header(&http_header::CONNECTION));
    assert!(!should_proxy_header(&http_header::TRANSFER_ENCODING));
    assert!(!should_proxy_header(&http_header::ACCEPT_ENCODING));
    assert!(!should_proxy_header(&HeaderName::from_static("forwarded")));
    assert!(!should_proxy_header(&HeaderName::from_static("via")));
    assert!(!should_proxy_header(&HeaderName::from_static(
        "x-forwarded-for"
    )));
    assert!(!should_proxy_header(&HeaderName::from_static(
        "x-forwarded-host"
    )));
    assert!(!should_proxy_header(&HeaderName::from_static(
        "x-forwarded-proto"
    )));
    assert!(!should_proxy_header(&HeaderName::from_static(
        "x-forwarded-port"
    )));
    assert!(!should_proxy_header(&HeaderName::from_static("x-real-ip")));
}

#[test]
fn connection_scoped_header_names_parses_connection_tokens() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("keep-alive, x-foo"),
    );
    headers.append(http_header::CONNECTION, HeaderValue::from_static("x-bar"));
    let names = connection_scoped_header_names(&headers);
    assert!(names.contains(&http_header::HeaderName::from_static("keep-alive")));
    assert!(names.contains(&http_header::HeaderName::from_static("x-foo")));
    assert!(names.contains(&http_header::HeaderName::from_static("x-bar")));
}

#[test]
fn request_may_have_body_uses_method_and_headers() {
    let empty = HeaderMap::new();
    assert!(!request_may_have_body(&Method::GET, &empty));
    assert!(request_may_have_body(&Method::POST, &empty));

    let mut with_length = HeaderMap::new();
    with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("0"));
    assert!(!request_may_have_body(&Method::GET, &with_length));
    with_length.insert(http_header::CONTENT_LENGTH, HeaderValue::from_static("10"));
    assert!(request_may_have_body(&Method::GET, &with_length));
}

#[test]
fn parse_cors_allowed_origins_normalizes_and_deduplicates() {
    let parsed = parse_cors_allowed_origins(
        "https://EXAMPLE.com:443, http://127.0.0.1:8080, https://example.com",
    )
    .expect("parse should succeed");
    assert_eq!(
        parsed,
        vec![
            "https://example.com".to_string(),
            "http://127.0.0.1:8080".to_string(),
        ]
    );
}

#[test]
fn origin_allowed_accepts_loopback_and_configured_origins() {
    let configured = HashSet::from(["https://api.example.com".to_string()]);
    assert!(origin_allowed("http://127.0.0.1:60080", &configured));
    assert!(origin_allowed("https://api.example.com", &configured));
    assert!(!origin_allowed("https://evil.example.com", &configured));
}

#[test]
fn same_origin_settings_write_allows_missing_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_cross_site_without_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_matching_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:8080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_matching_origin_without_explicit_host_port() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_mismatched_origin() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_loopback_proxy_port_mismatch() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:60080"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_forwarded_host_match() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_allows_forwarded_port_for_non_default_origin_port() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com:8443"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-port"),
        HeaderValue::from_static("8443"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );
    assert!(is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_multi_hop_forwarded_host_chain() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("evil.example.com, proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn same_origin_settings_write_rejects_cross_site_request() {
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );
    assert!(!is_same_origin_settings_write(&headers));
}

#[test]
fn rewrite_proxy_location_path_strips_upstream_base_prefix() {
    let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    assert_eq!(
        rewrite_proxy_location_path("/gateway/v1/echo", &upstream_base),
        "/v1/echo"
    );
    assert_eq!(
        rewrite_proxy_location_path("/v1/echo", &upstream_base),
        "/v1/echo"
    );
}

#[test]
fn normalize_proxy_location_header_strips_upstream_base_prefix_for_absolute_redirect() {
    let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::LOCATION,
        HeaderValue::from_static("https://proxy.example.com/gateway/v1/echo?from=redirect"),
    );

    let normalized =
        normalize_proxy_location_header(StatusCode::TEMPORARY_REDIRECT, &headers, &upstream_base)
            .expect("normalize should succeed");
    assert_eq!(normalized.as_deref(), Some("/v1/echo?from=redirect"));
}

#[test]
fn normalize_proxy_location_header_strips_upstream_base_prefix_for_relative_redirect() {
    let upstream_base = Url::parse("https://proxy.example.com/gateway/").expect("valid base");
    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::LOCATION,
        HeaderValue::from_static("/gateway/v1/echo?from=redirect#frag"),
    );

    let normalized =
        normalize_proxy_location_header(StatusCode::TEMPORARY_REDIRECT, &headers, &upstream_base)
            .expect("normalize should succeed");
    assert_eq!(normalized.as_deref(), Some("/v1/echo?from=redirect#frag"));
}

#[tokio::test]
async fn proxy_openai_v1_forwards_headers_method_query_and_body() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::AUTHORIZATION,
        HeaderValue::from_static("Bearer test-token"),
    );
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("client.example.com"),
    );
    headers.insert(
        http_header::CONNECTION,
        HeaderValue::from_static("keep-alive, x-foo"),
    );
    headers.insert(
        http_header::HeaderName::from_static("x-foo"),
        HeaderValue::from_static("should-not-forward"),
    );
    headers.insert(
        http_header::HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.20"),
    );
    headers.insert(
        http_header::HeaderName::from_static("via"),
        HeaderValue::from_static("1.1 browser-proxy"),
    );

    let uri: Uri = "/v1/echo?foo=bar".parse().expect("valid uri");
    let response = proxy_openai_v1(
        State(state),
        OriginalUri(uri),
        Method::POST,
        headers,
        Body::from("hello-proxy"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get("x-upstream"),
        Some(&HeaderValue::from_static("ok"))
    );
    assert!(response.headers().contains_key(http_header::CONTENT_LENGTH));
    assert!(
        !response
            .headers()
            .contains_key(http_header::HeaderName::from_static("x-upstream-hop"))
    );
    assert!(
        !response
            .headers()
            .contains_key(http_header::HeaderName::from_static("via"))
    );
    assert!(
        !response
            .headers()
            .contains_key(http_header::HeaderName::from_static("forwarded"))
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    assert_eq!(payload["method"], "POST");
    assert_eq!(payload["path"], "/v1/echo");
    assert_eq!(payload["query"], "foo=bar");
    assert_eq!(payload["authorization"], "Bearer test-token");
    assert_ne!(payload["hostHeader"], "client.example.com");
    assert_eq!(payload["connectionSeen"], false);
    assert_eq!(payload["xFooSeen"], false);
    assert_eq!(payload["xForwardedForSeen"], false);
    assert_eq!(payload["forwardedSeen"], false);
    assert_eq!(payload["viaSeen"], false);
    assert_eq!(payload["body"], "hello-proxy");

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_model_settings_api_reads_and_persists_updates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(initial) = get_settings(State(state.clone()))
        .await
        .expect("get settings should succeed");
    assert!(!initial.proxy.hijack_enabled);
    assert!(!initial.proxy.merge_upstream_enabled);
    assert_eq!(
        initial.proxy.fast_mode_rewrite_mode,
        ProxyFastModeRewriteMode::Disabled
    );
    assert_eq!(initial.proxy.models.len(), PROXY_PRESET_MODEL_IDS.len());
    assert_eq!(
        initial.proxy.enabled_models,
        PROXY_PRESET_MODEL_IDS
            .iter()
            .map(|id| id.to_string())
            .collect::<Vec<_>>()
    );

    let Json(updated) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: ProxyFastModeRewriteMode::FillMissing,
            enabled_models: vec!["gpt-5.2-codex".to_string(), "unknown-model".to_string()],
        }),
    )
    .await
    .expect("put settings should succeed");
    assert!(updated.hijack_enabled);
    assert!(updated.merge_upstream_enabled);
    assert_eq!(
        updated.fast_mode_rewrite_mode,
        ProxyFastModeRewriteMode::FillMissing
    );
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);

    let persisted = load_proxy_model_settings(&state.pool)
        .await
        .expect("settings should persist");
    assert!(persisted.hijack_enabled);
    assert!(persisted.merge_upstream_enabled);
    assert_eq!(
        persisted.fast_mode_rewrite_mode,
        ProxyFastModeRewriteMode::FillMissing
    );
    assert_eq!(
        persisted.enabled_preset_models,
        vec!["gpt-5.2-codex".to_string()]
    );

    let Json(normalized) = put_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: false,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: ProxyFastModeRewriteMode::ForcePriority,
            enabled_models: Vec::new(),
        }),
    )
    .await
    .expect("put settings should normalize payload");
    assert!(!normalized.hijack_enabled);
    assert!(!normalized.merge_upstream_enabled);
    assert_eq!(
        normalized.fast_mode_rewrite_mode,
        ProxyFastModeRewriteMode::ForcePriority
    );
    assert!(normalized.enabled_models.is_empty());
}

#[tokio::test]
async fn ensure_schema_adds_fast_mode_rewrite_mode_with_disabled_default() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");

    sqlx::query(
        r#"
        CREATE TABLE proxy_model_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            hijack_enabled INTEGER NOT NULL DEFAULT 0,
            merge_upstream_enabled INTEGER NOT NULL DEFAULT 0,
            enabled_preset_models_json TEXT,
            preset_models_migrated INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy proxy_model_settings table");

    sqlx::query(
        r#"
        INSERT INTO proxy_model_settings (
            id,
            hijack_enabled,
            merge_upstream_enabled,
            enabled_preset_models_json,
            preset_models_migrated
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .bind(1_i64)
    .bind(0_i64)
    .bind(
        serde_json::to_string(&default_enabled_preset_models())
            .expect("serialize default enabled models"),
    )
    .bind(1_i64)
    .execute(&pool)
    .await
    .expect("insert legacy proxy_model_settings row");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert_eq!(
        settings.fast_mode_rewrite_mode,
        ProxyFastModeRewriteMode::Disabled
    );
}

#[tokio::test]
async fn ensure_schema_appends_new_proxy_models_when_enabled_list_matches_legacy_default() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.4".to_string())
    );
    assert!(
        settings
            .enabled_preset_models
            .contains(&"gpt-5.4-pro".to_string())
    );
}

#[tokio::test]
async fn ensure_schema_does_not_append_new_proxy_models_when_enabled_list_is_custom() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let custom_enabled = vec!["gpt-5.2-codex".to_string()];
    let custom_enabled_json =
        serde_json::to_string(&custom_enabled).expect("serialize custom enabled list");
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(custom_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force custom enabled preset models");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert_eq!(settings.enabled_preset_models, custom_enabled);
}

#[tokio::test]
async fn ensure_schema_allows_opting_out_of_new_proxy_models_after_migration() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    let legacy_enabled = LEGACY_PROXY_PRESET_MODEL_IDS
        .iter()
        .map(|id| (*id).to_string())
        .collect::<Vec<_>>();
    let legacy_enabled_json =
        serde_json::to_string(&legacy_enabled).expect("serialize legacy enabled list");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind(&legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models");

    ensure_schema(&pool)
        .await
        .expect("ensure schema migration run");
    let migrated = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after migration");
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.4".to_string())
    );
    assert!(
        migrated
            .enabled_preset_models
            .contains(&"gpt-5.4-pro".to_string())
    );

    // User explicitly removes the new models after migration; schema re-run should not
    // force them back in.
    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 1
        WHERE id = ?2
        "#,
    )
    .bind(&legacy_enabled_json)
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force legacy enabled preset models after migration");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings after opt-out");
    assert_eq!(settings.enabled_preset_models, legacy_enabled);
}

#[tokio::test]
async fn ensure_schema_marks_proxy_preset_models_migrated_when_enabled_list_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE proxy_model_settings
        SET enabled_preset_models_json = ?1,
            preset_models_migrated = 0
        WHERE id = ?2
        "#,
    )
    .bind("[]")
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("force empty enabled preset models list");

    ensure_schema(&pool).await.expect("ensure schema rerun");

    let settings = load_proxy_model_settings(&pool)
        .await
        .expect("load proxy model settings");
    assert!(
        settings.enabled_preset_models.is_empty(),
        "empty enabled list should be preserved"
    );

    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT preset_models_migrated
        FROM proxy_model_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(PROXY_MODEL_SETTINGS_SINGLETON_ID)
    .fetch_one(&pool)
    .await
    .expect("read migration flag");
    assert_eq!(migrated, 1);
}

#[tokio::test]
async fn proxy_model_settings_api_rejects_cross_origin_writes() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );

    let err = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect_err("cross-origin write should be rejected");

    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_model_settings_api_rejects_cross_site_request() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://evil.example.com"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("cross-site"),
    );

    let err = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect_err("cross-site request should be rejected");

    assert_eq!(err.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_loopback_proxy_origin_mismatch() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("http://127.0.0.1:60080"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("loopback proxied write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_forwarded_host_origin_match() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("forwarded host write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_forwarded_port_non_default_origin_port() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("127.0.0.1:8080"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com:8443"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-host"),
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-proto"),
        HeaderValue::from_static("https"),
    );
    headers.insert(
        HeaderName::from_static("x-forwarded-port"),
        HeaderValue::from_static("8443"),
    );
    headers.insert(
        HeaderName::from_static("sec-fetch-site"),
        HeaderValue::from_static("same-origin"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("forwarded port write should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn proxy_model_settings_api_allows_matching_origin_without_explicit_host_port() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http_header::HOST,
        HeaderValue::from_static("proxy.example.com"),
    );
    headers.insert(
        http_header::ORIGIN,
        HeaderValue::from_static("https://proxy.example.com"),
    );

    let Json(updated) = put_proxy_settings(
        State(state),
        headers,
        Json(ProxyModelSettingsUpdateRequest {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_models: vec!["gpt-5.2-codex".to_string()],
        }),
    )
    .await
    .expect("same-origin write without explicit host port should be allowed");

    assert!(updated.hijack_enabled);
    assert!(!updated.merge_upstream_enabled);
    assert_eq!(updated.enabled_models, vec!["gpt-5.2-codex".to_string()]);
}

#[tokio::test]
async fn forward_proxy_live_stats_returns_fixed_24_hour_buckets_with_zero_fill() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(settings_response) = put_forward_proxy_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(ForwardProxySettingsUpdateRequest {
            proxy_urls: vec!["socks5://127.0.0.1:1080".to_string()],
            subscription_urls: vec![],
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        }),
    )
    .await
    .expect("put forward proxy settings should succeed");

    let direct_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_DIRECT)
        .map(|node| node.key.clone())
        .expect("direct node should exist");
    let direct_weight = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_DIRECT)
        .map(|node| node.weight)
        .expect("direct node weight should exist");
    let manual_key = settings_response
        .nodes
        .iter()
        .find(|node| node.source == FORWARD_PROXY_SOURCE_MANUAL)
        .map(|node| node.key.clone())
        .expect("manual node should exist");

    let now = Utc::now();
    let range_end_epoch = align_bucket_epoch(now.timestamp(), 3600, 0) + 3600;
    let range_start_epoch = range_end_epoch - 24 * 3600;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch - 3600,
        4,
        0.25,
        0.42,
        0.34,
        0.35,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch + 5 * 3600,
        2,
        0.45,
        0.82,
        0.61,
        0.80,
    )
    .await;
    seed_forward_proxy_weight_bucket_at(
        &state.pool,
        &manual_key,
        range_start_epoch + 10 * 3600,
        1,
        1.20,
        1.20,
        1.20,
        1.20,
    )
    .await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        now - ChronoDuration::minutes(12),
        true,
    )
    .await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        now - ChronoDuration::hours(1) - ChronoDuration::minutes(8),
        false,
    )
    .await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &manual_key,
        now - ChronoDuration::hours(30),
        true,
    )
    .await;
    seed_forward_proxy_attempt_at(
        &state.pool,
        &direct_key,
        now - ChronoDuration::minutes(40),
        true,
    )
    .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state.clone()))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(response.nodes.len(), 2);
    assert_eq!(response.range_end, response.nodes[0].last24h[23].bucket_end);
    assert_eq!(
        response.range_start,
        response.nodes[0].last24h[0].bucket_start
    );

    for node in &response.nodes {
        assert_eq!(
            node.last24h.len(),
            24,
            "node {} should include fixed 24 buckets",
            node.key
        );
        assert_eq!(
            node.weight24h.len(),
            24,
            "node {} should include fixed 24 weight buckets",
            node.key
        );
    }

    let manual = response
        .nodes
        .iter()
        .find(|node| node.key == manual_key)
        .expect("manual node should be present");
    let manual_success_total: i64 = manual
        .last24h
        .iter()
        .map(|bucket| bucket.success_count)
        .sum();
    let manual_failure_total: i64 = manual
        .last24h
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum();
    let manual_zero_buckets = manual
        .last24h
        .iter()
        .filter(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
        .count();
    assert_eq!(
        manual_success_total, 1,
        "out-of-range attempts should be excluded"
    );
    assert!(
        manual_failure_total >= 1,
        "expected at least one in-range failure attempt"
    );
    assert!(
        manual_zero_buckets >= 21,
        "expected most buckets to be zero-filled, got {manual_zero_buckets}"
    );
    assert!(
        manual
            .weight24h
            .iter()
            .any(|bucket| bucket.sample_count == 0 && (bucket.last_weight - 0.35).abs() < 1e-6)
    );

    let sampled_bucket_index = manual
        .weight24h
        .iter()
        .position(|bucket| {
            bucket.sample_count == 2
                && (bucket.min_weight - 0.45).abs() < 1e-6
                && (bucket.max_weight - 0.82).abs() < 1e-6
                && (bucket.avg_weight - 0.61).abs() < 1e-6
                && (bucket.last_weight - 0.80).abs() < 1e-6
        })
        .expect("expected sampled manual weight bucket with aggregated stats");
    let sampled_bucket_carry = manual
        .weight24h
        .get(sampled_bucket_index + 1)
        .expect("expected carry-forward bucket after sampled manual weight bucket");
    assert_eq!(sampled_bucket_carry.sample_count, 0);
    assert!((sampled_bucket_carry.last_weight - 0.80).abs() < 1e-6);

    let recovered_bucket_index = manual
        .weight24h
        .iter()
        .position(|bucket| {
            bucket.sample_count == 1
                && (bucket.min_weight - 1.20).abs() < 1e-6
                && (bucket.max_weight - 1.20).abs() < 1e-6
                && (bucket.avg_weight - 1.20).abs() < 1e-6
                && (bucket.last_weight - 1.20).abs() < 1e-6
        })
        .expect("expected sampled manual weight bucket with recovered last value");
    let recovered_bucket_carry = manual
        .weight24h
        .get(recovered_bucket_index + 1)
        .expect("expected carry-forward bucket after recovered manual weight bucket");
    assert_eq!(recovered_bucket_carry.sample_count, 0);
    assert!((recovered_bucket_carry.last_weight - 1.20).abs() < 1e-6);
    let direct = response
        .nodes
        .iter()
        .find(|node| node.key == direct_key)
        .expect("direct node should be present");
    let direct_success_total: i64 = direct
        .last24h
        .iter()
        .map(|bucket| bucket.success_count)
        .sum();
    let direct_failure_total: i64 = direct
        .last24h
        .iter()
        .map(|bucket| bucket.failure_count)
        .sum();
    assert_eq!(direct_success_total, 1);
    assert_eq!(direct_failure_total, 0);
    assert!(direct.weight24h.iter().all(
        |bucket| bucket.sample_count == 0 && (bucket.last_weight - direct_weight).abs() < 1e-6
    ));

    let display_names = response
        .nodes
        .iter()
        .map(|node| node.display_name.as_str())
        .collect::<Vec<_>>();
    let mut sorted_display_names = display_names.clone();
    sorted_display_names.sort();
    assert_eq!(
        display_names, sorted_display_names,
        "live stats nodes should stay display-name sorted"
    );
}

#[tokio::test]
async fn forward_proxy_live_stats_keeps_direct_node_and_zero_metrics_when_no_attempts() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(response) = fetch_forward_proxy_live_stats(State(state))
        .await
        .expect("fetch forward proxy live stats should succeed");

    assert_eq!(response.bucket_seconds, 3600);
    assert_eq!(
        response.nodes.len(),
        1,
        "default runtime should only include direct node"
    );
    let direct = &response.nodes[0];
    assert_eq!(direct.source, FORWARD_PROXY_SOURCE_DIRECT);
    assert_eq!(direct.stats.one_minute.attempts, 0);
    assert_eq!(direct.stats.one_day.attempts, 0);
    assert_eq!(direct.last24h.len(), 24);
    assert_eq!(direct.weight24h.len(), 24);
    assert!(
        direct
            .last24h
            .iter()
            .all(|bucket| bucket.success_count == 0 && bucket.failure_count == 0)
    );
    assert!(direct.weight24h.iter().all(
        |bucket| bucket.sample_count == 0 && (bucket.last_weight - direct.weight).abs() < 1e-6
    ));
}

#[tokio::test]
async fn upsert_forward_proxy_weight_hourly_bucket_keeps_latest_sample_weight() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;
    let proxy_key = "manual://latest-sample-weight";
    let bucket_start_epoch = align_bucket_epoch(Utc::now().timestamp(), 3600, 0);

    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        0.90,
        1_000_000,
    )
    .await
    .expect("seed first weight sample");
    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        1.10,
        2_000_000,
    )
    .await
    .expect("seed second weight sample");
    upsert_forward_proxy_weight_hourly_bucket(
        &state.pool,
        proxy_key,
        bucket_start_epoch,
        0.70,
        1_500_000,
    )
    .await
    .expect("seed out-of-order weight sample");

    let row = sqlx::query(
        r#"
        SELECT
            sample_count,
            min_weight,
            max_weight,
            avg_weight,
            last_weight,
            last_sample_epoch_us
        FROM forward_proxy_weight_hourly
        WHERE proxy_key = ?1 AND bucket_start_epoch = ?2
        "#,
    )
    .bind(proxy_key)
    .bind(bucket_start_epoch)
    .fetch_one(&state.pool)
    .await
    .expect("fetch aggregated weight bucket");

    assert_eq!(
        row.try_get::<i64, _>("sample_count").expect("sample_count"),
        3
    );
    assert!((row.try_get::<f64, _>("min_weight").expect("min_weight") - 0.70).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("max_weight").expect("max_weight") - 1.10).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("avg_weight").expect("avg_weight") - 0.90).abs() < 1e-6);
    assert!((row.try_get::<f64, _>("last_weight").expect("last_weight") - 1.10).abs() < 1e-6);
    assert_eq!(
        row.try_get::<i64, _>("last_sample_epoch_us")
            .expect("last_sample_epoch_us"),
        2_000_000
    );
}

#[tokio::test]
async fn pricing_settings_api_reads_and_persists_updates() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(initial) = get_settings(State(state.clone()))
        .await
        .expect("get settings should succeed");
    assert!(!initial.pricing.entries.is_empty());
    assert!(
        initial
            .pricing
            .entries
            .iter()
            .any(|entry| entry.model == "gpt-5.2-codex")
    );

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-ci".to_string(),
            entries: vec![PricingEntry {
                model: "gpt-5.2-codex".to_string(),
                input_per_1m: 8.8,
                output_per_1m: 18.8,
                cache_input_per_1m: Some(0.88),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            }],
        }),
    )
    .await
    .expect("put pricing settings should succeed");

    assert_eq!(updated.catalog_version, "custom-ci");
    assert_eq!(updated.entries.len(), 1);
    assert_eq!(updated.entries[0].model, "gpt-5.2-codex");
    assert_eq!(updated.entries[0].input_per_1m, 8.8);

    let persisted = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing settings should persist");
    assert_eq!(persisted.version, "custom-ci");
    assert_eq!(persisted.models.len(), 1);
    let pricing = persisted
        .models
        .get("gpt-5.2-codex")
        .expect("gpt-5.2-codex should persist");
    assert_eq!(pricing.input_per_1m, 8.8);
    assert_eq!(pricing.output_per_1m, 18.8);
}

#[tokio::test]
async fn pricing_settings_api_keeps_empty_catalog_after_reload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let Json(updated) = put_pricing_settings(
        State(state.clone()),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "custom-empty".to_string(),
            entries: vec![],
        }),
    )
    .await
    .expect("put pricing settings should allow empty catalog");

    assert_eq!(updated.catalog_version, "custom-empty");
    assert!(updated.entries.is_empty());

    let first_reload = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should load after update");
    assert_eq!(first_reload.version, "custom-empty");
    assert!(first_reload.models.is_empty());

    let second_reload = load_pricing_catalog(&state.pool)
        .await
        .expect("pricing catalog should stay empty across reloads");
    assert_eq!(second_reload.version, "custom-empty");
    assert!(second_reload.models.is_empty());
}

#[tokio::test]
async fn pricing_settings_api_rejects_invalid_payload() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.example.com/").expect("valid upstream base url"),
    )
    .await;

    let err = put_pricing_settings(
        State(state),
        HeaderMap::new(),
        Json(PricingSettingsUpdateRequest {
            catalog_version: "   ".to_string(),
            entries: vec![],
        }),
    )
    .await
    .expect_err("blank catalog version should be rejected");

    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn seed_default_pricing_catalog_migrates_legacy_file_when_present() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-custom-v1",
  "models": {
"gpt-legacy": {
  "input_per_1m": 9.9,
  "output_per_1m": 19.9,
  "cache_input_per_1m": 0.99,
  "reasoning_per_1m": null
}
  }
}"#,
    )
    .expect("write legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog from legacy file");

    let _ = fs::remove_file(&legacy_path);

    let migrated = load_pricing_catalog(&pool)
        .await
        .expect("load migrated pricing catalog");
    assert_eq!(migrated.version, "legacy-custom-v1");
    assert_eq!(migrated.models.len(), 1);
    let model = migrated
        .models
        .get("gpt-legacy")
        .expect("legacy model should be migrated");
    assert_eq!(model.input_per_1m, 9.9);
    assert_eq!(model.output_per_1m, 19.9);
    assert_eq!(model.cache_input_per_1m, Some(0.99));
    assert_eq!(model.source, "custom");
}

#[tokio::test]
async fn seed_default_pricing_catalog_falls_back_when_legacy_file_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");
    sqlx::query("DELETE FROM pricing_settings_meta")
        .execute(&pool)
        .await
        .expect("clear pricing meta");
    sqlx::query("DELETE FROM pricing_settings_models")
        .execute(&pool)
        .await
        .expect("clear pricing models");

    let legacy_path = env::temp_dir().join(format!(
        "codex-vibe-monitor-pricing-legacy-empty-{}.json",
        NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(
        &legacy_path,
        r#"{
  "version": "legacy-empty",
  "models": {}
}"#,
    )
    .expect("write empty legacy pricing catalog");

    seed_default_pricing_catalog_with_legacy_path(&pool, Some(&legacy_path))
        .await
        .expect("seed pricing catalog should fall back to defaults");

    let _ = fs::remove_file(&legacy_path);

    let seeded = load_pricing_catalog(&pool)
        .await
        .expect("load seeded pricing catalog");
    assert_eq!(seeded.version, DEFAULT_PRICING_CATALOG_VERSION);
    assert!(
        seeded.models.contains_key("gpt-5.2-codex"),
        "default pricing catalog should be seeded"
    );
}

#[tokio::test]
async fn seed_default_pricing_catalog_auto_inserts_new_models_for_legacy_default_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("downgrade pricing catalog version for test");
    sqlx::query(
        r#"
        DELETE FROM pricing_settings_models
        WHERE model IN ('gpt-5.4', 'gpt-5.4-pro')
        "#,
    )
    .execute(&pool)
    .await
    .expect("delete new pricing models for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    assert!(catalog.models.contains_key("gpt-5.4"));
    assert!(catalog.models.contains_key("gpt-5.4-pro"));
}

#[tokio::test]
async fn seed_default_pricing_catalog_normalizes_gpt_5_3_codex_source_for_legacy_default_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    save_pricing_catalog(&pool, &default_pricing_catalog())
        .await
        .expect("seed default pricing catalog");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("downgrade catalog version for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET source = 'temporary'
        WHERE model = 'gpt-5.3-codex'
        "#,
    )
    .execute(&pool)
    .await
    .expect("force legacy gpt-5.3-codex source");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog");
    let pricing = catalog
        .models
        .get("gpt-5.3-codex")
        .expect("gpt-5.3-codex pricing present");
    assert_eq!(pricing.source, "official");
}

#[tokio::test]
async fn seed_default_pricing_catalog_does_not_auto_insert_new_models_for_custom_catalog_version() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind("custom-ci")
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set custom pricing catalog version for test");
    sqlx::query(
        r#"
        DELETE FROM pricing_settings_models
        WHERE model IN ('gpt-5.4', 'gpt-5.4-pro')
        "#,
    )
    .execute(&pool)
    .await
    .expect("delete new pricing models for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    assert!(!catalog.models.contains_key("gpt-5.4"));
    assert!(!catalog.models.contains_key("gpt-5.4-pro"));
}

#[tokio::test]
async fn seed_default_pricing_catalog_does_not_override_existing_pricing_for_new_models() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    // Simulate a legacy default catalog version so startup seeding will call
    // ensure_pricing_models_present, which must not overwrite existing rows.
    sqlx::query(
        r#"
        UPDATE pricing_settings_meta
        SET catalog_version = ?1
        WHERE id = ?2
        "#,
    )
    .bind(LEGACY_DEFAULT_PRICING_CATALOG_VERSION)
    .bind(PRICING_SETTINGS_SINGLETON_ID)
    .execute(&pool)
    .await
    .expect("set legacy pricing catalog version for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = ?1,
            output_per_1m = ?2,
            cache_input_per_1m = ?3,
            source = 'custom'
        WHERE model = 'gpt-5.4'
        "#,
    )
    .bind(99.0)
    .bind(199.0)
    .bind(Some(9.9))
    .execute(&pool)
    .await
    .expect("override gpt-5.4 pricing for test");

    sqlx::query(
        r#"
        UPDATE pricing_settings_models
        SET input_per_1m = ?1,
            output_per_1m = ?2,
            source = 'custom'
        WHERE model = 'gpt-5.4-pro'
        "#,
    )
    .bind(88.0)
    .bind(188.0)
    .execute(&pool)
    .await
    .expect("override gpt-5.4-pro pricing for test");

    let catalog = load_pricing_catalog(&pool)
        .await
        .expect("load pricing catalog should succeed");
    let gpt_5_4 = catalog.models.get("gpt-5.4").expect("gpt-5.4 should exist");
    assert_eq!(gpt_5_4.input_per_1m, 99.0);
    assert_eq!(gpt_5_4.output_per_1m, 199.0);
    assert_eq!(gpt_5_4.cache_input_per_1m, Some(9.9));
    assert_eq!(gpt_5_4.source, "custom");

    let gpt_5_4_pro = catalog
        .models
        .get("gpt-5.4-pro")
        .expect("gpt-5.4-pro should exist");
    assert_eq!(gpt_5_4_pro.input_per_1m, 88.0);
    assert_eq!(gpt_5_4_pro.output_per_1m, 188.0);
    assert_eq!(gpt_5_4_pro.cache_input_per_1m, None);
    assert_eq!(gpt_5_4_pro.source, "custom");
}

#[tokio::test]
async fn proxy_openai_v1_models_passthrough_when_hijack_disabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode upstream payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(
        ids,
        vec!["upstream-model-a".to_string(), "gpt-5.2-codex".to_string()]
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_returns_preset_when_hijack_enabled_without_merge() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_preset_models: vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(PROXY_MODEL_MERGE_STATUS_HEADER)
            .is_none()
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(
        ids,
        vec!["gpt-5.3-codex".to_string(), "gpt-5.2".to_string()]
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_returns_gpt_5_4_models_when_enabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: false,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_preset_models: vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode hijacked payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.4".to_string(), "gpt-5.4-pro".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_merges_upstream_when_enabled() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_preset_models: vec![
                "gpt-5.2-codex".to_string(),
                "gpt-5.1-codex-mini".to_string(),
            ],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_SUCCESS))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode merged payload");
    let ids = extract_model_ids(&payload);

    assert!(ids.contains(&"upstream-model-a".to_string()));
    assert!(ids.contains(&"gpt-5.2-codex".to_string()));
    assert!(ids.contains(&"gpt-5.1-codex-mini".to_string()));
    assert!(!ids.contains(&"gpt-5.3-codex".to_string()));
    assert_eq!(
        ids.iter()
            .filter(|id| id.as_str() == "gpt-5.2-codex")
            .count(),
        1
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_falls_back_to_preset_when_merge_upstream_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        *settings = ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        };
    }

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models?mode=error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_models_falls_back_when_merge_body_decode_times_out() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings {
            hijack_enabled: true,
            merge_upstream_enabled: true,
            fast_mode_rewrite_mode: DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
            enabled_preset_models: vec!["gpt-5.1-codex-mini".to_string()],
        })),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    let started = Instant::now();
    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/models?mode=slow-body".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "merge fallback should return quickly when decode times out"
    );
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(PROXY_MODEL_MERGE_STATUS_HEADER),
        Some(&HeaderValue::from_static(PROXY_MODEL_MERGE_STATUS_FAILED))
    );
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read fallback response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode fallback payload");
    let ids = extract_model_ids(&payload);
    assert_eq!(ids, vec!["gpt-5.1-codex-mini".to_string()]);

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_preserves_streaming_response() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(http_header::CONTENT_TYPE),
        Some(&HeaderValue::from_static("text/event-stream"))
    );

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read stream body");
    assert_eq!(&body[..], b"chunk-achunk-b");

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_returns_bad_gateway_when_first_stream_chunk_fails() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream-first-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("upstream stream error before first chunk")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_propagates_stream_error_after_first_chunk() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/stream-mid-error".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("mid-stream upstream failure should surface to downstream");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_preserves_redirect_without_following() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/redirect".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(
        response.headers().get(http_header::LOCATION),
        Some(&HeaderValue::from_static("/v1/echo?from=redirect"))
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_blocks_cross_origin_redirect() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/redirect-external".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("cross-origin redirect is not allowed")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_target_persists_record_on_redirect_rewrite_error() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        source: String,
        status: Option<String>,
        error_message: Option<String>,
        t_total_ms: Option<f64>,
        t_req_read_ms: Option<f64>,
        t_req_parse_ms: Option<f64>,
        t_upstream_connect_ms: Option<f64>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(
            r#"{"model":"gpt-5.2","stream":false,"messages":[{"role":"user","content":"hi"}]}"#,
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("cross-origin redirect is not allowed")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT source, status, error_message, t_total_ms, t_req_read_ms, t_req_parse_ms, t_upstream_connect_ms
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.source, SOURCE_PROXY);
    assert_eq!(row.status.as_deref(), Some("http_502"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("cross-origin redirect is not allowed"))
    );
    assert!(row.t_total_ms.is_some_and(|v| v > 0.0));
    assert!(row.t_req_read_ms.is_some_and(|v| v >= 0.0));
    assert!(row.t_req_parse_ms.is_some_and(|v| v >= 0.0));
    assert!(row.t_upstream_connect_ms.is_some_and(|v| v >= 0.0));

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_persist_and_broadcast_emits_records_summary_and_quota() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;

    let mut rx = state.broadcaster.subscribe();
    let invoke_id = "proxy-sse-broadcast-success";
    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &now_local),
    )
    .await
    .expect("persist+broadcast should succeed");

    let mut saw_record = false;
    let mut captured_record: Option<ApiInvocation> = None;
    let mut saw_quota = false;
    let mut summary_windows = HashSet::new();
    let expected_summary_windows = summary_broadcast_specs().len();
    for _ in 0..16 {
        let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("timed out waiting for proxy broadcast event")
            .expect("broadcast channel should stay open");
        match payload {
            BroadcastPayload::Records { records } => {
                if let Some(record) = records
                    .into_iter()
                    .find(|record| record.invoke_id == invoke_id)
                {
                    saw_record = true;
                    captured_record = Some(record);
                }
            }
            BroadcastPayload::Summary { window, summary } => {
                summary_windows.insert(window.clone());
                if window == "all" {
                    assert_eq!(summary.total_count, 1);
                }
            }
            BroadcastPayload::Quota { snapshot } => {
                saw_quota = true;
                assert_eq!(snapshot.total_requests, 9);
            }
            BroadcastPayload::Version { .. } => {}
        }

        if saw_record && saw_quota && summary_windows.len() == expected_summary_windows {
            break;
        }
    }

    assert!(saw_record, "records payload should be broadcast");
    assert!(saw_quota, "quota payload should be broadcast");
    assert_eq!(
        summary_windows.len(),
        expected_summary_windows,
        "all summary windows should be broadcast"
    );
    let record = captured_record.expect("target records payload should include invoke id");
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-broadcast-1"));
    assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
    assert!(record.failure_kind.is_none());
}

#[tokio::test]
async fn proxy_capture_persist_and_broadcast_skips_duplicate_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    let invoke_id = "proxy-sse-broadcast-duplicate";
    let mut rx = state.broadcaster.subscribe();

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("initial persist+broadcast should succeed");

    drain_broadcast_messages(&mut rx).await;

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record(invoke_id, &occurred_at),
    )
    .await
    .expect("duplicate persist should not fail");

    let deadline = Instant::now() + Duration::from_millis(400);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Ok(BroadcastPayload::Records { records })) => {
                assert!(
                    records.iter().all(|record| record.invoke_id != invoke_id),
                    "duplicate insert should not emit records payload for the same invoke_id"
                );
            }
            Ok(Ok(_)) => continue,
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => continue,
        }
    }
}

#[tokio::test]
async fn fetch_and_store_skips_summary_and_quota_collection_when_broadcast_state_disabled() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let now_local = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &now_local).await;

    let publish = fetch_and_store(state.as_ref(), false, false)
        .await
        .expect("fetch_and_store should succeed");

    assert!(publish.summaries.is_empty());
    assert!(publish.quota_snapshot.is_none());
    assert!(!publish.collected_broadcast_state);
}

#[test]
fn should_collect_late_broadcast_state_when_subscribers_arrive_mid_poll() {
    assert!(should_collect_late_broadcast_state(1, false));
    assert!(!should_collect_late_broadcast_state(0, false));
    assert!(!should_collect_late_broadcast_state(1, true));
}

#[tokio::test]
async fn broadcast_summary_if_changed_skips_duplicate_payloads() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let first = StatsResponse {
        total_count: 1,
        success_count: 1,
        failure_count: 0,
        total_cost: 0.5,
        total_tokens: 42,
    };

    assert!(
        broadcast_summary_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            "1d",
            first.clone(),
        )
        .await
        .expect("first summary broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for first summary payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Summary { window, summary } => {
            assert_eq!(window, "1d");
            assert_eq!(summary, first);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    assert!(
        !broadcast_summary_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            "1d",
            first.clone(),
        )
        .await
        .expect("duplicate summary broadcast should succeed")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );

    let updated = StatsResponse {
        total_count: 2,
        ..first
    };
    assert!(
        broadcast_summary_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            "1d",
            updated.clone(),
        )
        .await
        .expect("changed summary broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for updated summary payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Summary { window, summary } => {
            assert_eq!(window, "1d");
            assert_eq!(summary, updated);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_quota_if_changed_skips_duplicate_payloads() {
    let state = test_state_with_openai_base(
        Url::parse("https://example-upstream.invalid/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    let first = QuotaSnapshotResponse {
        captured_at: "2026-03-07 10:00:00".to_string(),
        amount_limit: Some(100.0),
        used_amount: Some(10.0),
        remaining_amount: Some(90.0),
        period: Some("monthly".to_string()),
        period_reset_time: Some("2026-04-01 00:00:00".to_string()),
        expire_time: None,
        is_active: true,
        total_cost: 10.0,
        total_requests: 9,
        total_tokens: 150,
        last_request_time: Some("2026-03-07 10:00:00".to_string()),
        billing_type: Some("prepaid".to_string()),
        remaining_count: Some(91),
        used_count: Some(9),
        sub_type_name: Some("unit".to_string()),
    };

    assert!(
        broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            first.clone(),
        )
        .await
        .expect("first quota broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for first quota payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Quota { snapshot } => {
            assert_eq!(*snapshot, first);
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    assert!(
        !broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            first.clone(),
        )
        .await
        .expect("duplicate quota broadcast should succeed")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .is_err()
    );

    let updated = QuotaSnapshotResponse {
        total_requests: 10,
        ..first
    };
    assert!(
        broadcast_quota_if_changed(
            &state.broadcaster,
            state.broadcast_state_cache.as_ref(),
            updated.clone(),
        )
        .await
        .expect("changed quota broadcast should succeed")
    );

    let payload = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("timed out waiting for updated quota payload")
        .expect("broadcast should stay open");
    match payload {
        BroadcastPayload::Quota { snapshot } => {
            assert_eq!(*snapshot, updated);
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[tokio::test]
async fn read_request_body_timeout_returns_408() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = test_state_with_openai_base_body_limit_and_read_timeout(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(50),
    )
    .await;

    let slow_body = stream::unfold(0u8, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<Bytes, Infallible>(Bytes::from_static(br#"{}"#)), 1))
            }
            _ => None,
        }
    });

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(slow_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body read timed out")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_read_timeout]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_read_timeout")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_client_body_disconnect_returns_400_with_failure_kind() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let disconnected_body = stream::iter(vec![Err::<Bytes, io::Error>(io::Error::new(
        io::ErrorKind::BrokenPipe,
        "client disconnected",
    ))]);

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(disconnected_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("failed to read request body stream")
    );

    let row = sqlx::query_as::<_, PersistedRow>(
        r#"
        SELECT status, error_message, payload
        FROM codex_invocations
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(&state.pool)
    .await
    .expect("query capture record")
    .expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("failed"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[request_body_stream_error_client_closed]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("request_body_stream_error_client_closed")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn capture_target_stream_error_emits_failure_kind_and_persists() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        status: Option<String>,
        error_message: Option<String>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.2-codex",
        "stream": true,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let err = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect_err("mid-stream upstream failure should surface to downstream");
    assert!(
        err.to_string().contains("upstream stream error"),
        "unexpected stream error text: {err}"
    );

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT status, error_message, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");

    assert_eq!(row.status.as_deref(), Some("http_200"));
    assert!(
        row.error_message
            .as_deref()
            .is_some_and(|msg| msg.contains("[upstream_stream_error]"))
    );
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(
        payload_json["failureKind"].as_str(),
        Some("upstream_stream_error")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_target_fill_missing_rewrites_priority_for_responses() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
    }

    let (upstream_base, captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.fast_mode_rewrite_mode = ProxyFastModeRewriteMode::FillMissing;
    }

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hello"
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let captured = captured_requests.lock().await;
    let captured_request = captured
        .first()
        .cloned()
        .expect("upstream should receive a request body");
    drop(captured);
    assert_eq!(captured_request["service_tier"], "priority");
    assert!(captured_request.get("serviceTier").is_none());

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(payload_json["requestedServiceTier"], "priority");

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_target_force_priority_overrides_existing_chat_tier() {
    #[derive(sqlx::FromRow)]
    struct PersistedRow {
        payload: Option<String>,
    }

    let (upstream_base, captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.fast_mode_rewrite_mode = ProxyFastModeRewriteMode::ForcePriority;
    }

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex",
        "messages": [{"role": "user", "content": "hello"}]
    }))
    .expect("serialize request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/chat/completions".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let captured = captured_requests.lock().await;
    let captured_request = captured
        .first()
        .cloned()
        .expect("upstream should receive a request body");
    drop(captured);
    assert_eq!(captured_request["service_tier"], "priority");
    assert!(captured_request.get("serviceTier").is_none());

    let mut row: Option<PersistedRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedRow>(
            r#"
            SELECT payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("capture record should be persisted");
    let payload_json: Value = serde_json::from_str(
        row.payload
            .as_deref()
            .expect("capture payload should be present"),
    )
    .expect("decode capture payload");
    assert_eq!(payload_json["requestedServiceTier"], "priority");

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_target_compact_estimates_cost_and_flows_into_stats_without_rewrite() {
    #[derive(sqlx::FromRow)]
    struct PersistedCompactRow {
        endpoint: Option<String>,
        model: Option<String>,
        requested_service_tier: Option<String>,
        input_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        reasoning_tokens: Option<i64>,
        total_tokens: Option<i64>,
        cost: Option<f64>,
        price_version: Option<String>,
    }

    let (upstream_base, captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;
    {
        let mut settings = state.proxy_model_settings.write().await;
        settings.fast_mode_rewrite_mode = ProxyFastModeRewriteMode::ForcePriority;
    }
    {
        let mut pricing = state.pricing_catalog.write().await;
        *pricing = PricingCatalog {
            version: "compact-unit-test".to_string(),
            models: HashMap::from([(
                "gpt-5.1-codex-max".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: Some(0.5),
                    reasoning_per_1m: Some(7.0),
                    source: "custom".to_string(),
                },
            )]),
        };
    }

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "previous_response_id": "resp_prev_001",
        "input": [{
            "role": "user",
            "content": "compact this thread"
        }]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses/compact".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _response_body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy response body");
    let captured = captured_requests.lock().await;
    let captured_request = captured
        .first()
        .cloned()
        .expect("upstream should receive a compact request body");
    drop(captured);
    assert_eq!(captured_request["serviceTier"], "flex");
    assert!(captured_request.get("service_tier").is_none());

    let mut row: Option<PersistedCompactRow> = None;
    for _ in 0..20 {
        row = sqlx::query_as::<_, PersistedCompactRow>(
            r#"
            SELECT
                CASE WHEN json_valid(payload) THEN json_extract(payload, '$.endpoint') END AS endpoint,
                model,
                CASE
                  WHEN json_valid(payload) AND json_type(payload, '$.requestedServiceTier') = 'text'
                    THEN json_extract(payload, '$.requestedServiceTier')
                  WHEN json_valid(payload) AND json_type(payload, '$.requested_service_tier') = 'text'
                    THEN json_extract(payload, '$.requested_service_tier')
                END AS requested_service_tier,
                input_tokens,
                cache_input_tokens,
                output_tokens,
                reasoning_tokens,
                total_tokens,
                cost,
                price_version
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query compact capture record");
        if row.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = row.expect("compact capture record should be persisted");
    assert_eq!(row.endpoint.as_deref(), Some("/v1/responses/compact"));
    assert_eq!(row.model.as_deref(), Some("gpt-5.1-codex-max"));
    assert_eq!(row.requested_service_tier.as_deref(), Some("flex"));
    assert_eq!(row.input_tokens, Some(139));
    assert_eq!(row.cache_input_tokens, Some(11));
    assert_eq!(row.output_tokens, Some(438));
    assert_eq!(row.reasoning_tokens, Some(64));
    assert_eq!(row.total_tokens, Some(577));
    assert_eq!(row.price_version.as_deref(), Some("compact-unit-test"));
    assert_f64_close(row.cost.expect("compact cost should be present"), 0.0020235);

    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("compact fetch_stats should succeed");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.success_count, 1);
    assert_eq!(stats.failure_count, 0);
    assert_eq!(stats.total_tokens, 577);
    assert_f64_close(stats.total_cost, 0.0020235);

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: None,
        }),
    )
    .await
    .expect("compact fetch_summary should succeed");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.total_tokens, 577);
    assert_f64_close(summary.total_cost, 0.0020235);

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: None,
        }),
    )
    .await
    .expect("compact fetch_timeseries should succeed");
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        577
    );
    assert_f64_close(
        timeseries
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.0020235,
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_target_compact_uses_dedicated_handshake_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base_and_proxy_timeouts(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(100),
        Duration::from_millis(400),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.1-codex-max",
        "previous_response_id": "resp_prev_001",
        "input": [{"role": "user", "content": "compact this thread"}]
    }))
    .expect("serialize compact request body");

    let response = proxy_openai_v1(
        State(state),
        OriginalUri(
            "/v1/responses/compact?mode=delay"
                .parse()
                .expect("valid uri"),
        ),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_capture_target_responses_uses_default_handshake_timeout() {
    let (upstream_base, _captured_requests, upstream_handle) =
        spawn_capture_target_body_upstream().await;
    let state = test_state_with_openai_base_and_proxy_timeouts(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        DEFAULT_OPENAI_PROXY_MAX_REQUEST_BODY_BYTES,
        Duration::from_millis(100),
        Duration::from_millis(400),
        Duration::from_secs(DEFAULT_OPENAI_PROXY_REQUEST_READ_TIMEOUT_SECS),
    )
    .await;

    let request_body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hello"
    }))
    .expect("serialize responses request body");

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/responses?mode=delay".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from(request_body),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read proxy error body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_allows_slow_upload_with_short_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    let slow_chunks = stream::unfold(0u8, |state| async move {
        match state {
            0 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"hello-")), 1))
            }
            1 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"slow-")), 2))
            }
            2 => {
                tokio::time::sleep(Duration::from_millis(120)).await;
                Some((Ok::<_, Infallible>(Bytes::from_static(b"upload")), 3))
            }
            _ => None,
        }
    });

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/echo?mode=slow-upload".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from_stream(slow_chunks),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy response payload");
    assert_eq!(payload["query"], "mode=slow-upload");
    assert_eq!(payload["body"], "hello-slow-upload");

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_e2e_http_roundtrip() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/v1/echo?foo=e2e"))
        .header(http_header::AUTHORIZATION, "Bearer e2e-token")
        .body("hello-e2e")
        .send()
        .await
        .expect("send proxy request");

    assert_eq!(response.status(), StatusCode::CREATED);
    let payload: Value = response
        .json()
        .await
        .expect("decode proxied upstream payload");
    assert_eq!(payload["method"], "POST");
    assert_eq!(payload["path"], "/v1/echo");
    assert_eq!(payload["query"], "foo=e2e");
    assert_eq!(payload["authorization"], "Bearer e2e-token");
    assert_eq!(payload["body"], "hello-e2e");

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_e2e_stream_survives_short_request_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.request_timeout = Duration::from_millis(200);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    let app = Router::new()
        .route("/v1/*path", any(proxy_openai_v1))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy test server");
    let addr = listener.local_addr().expect("proxy test server addr");
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("proxy test server should run");
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/v1/slow-stream"))
        .send()
        .await
        .expect("send proxy stream request");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.bytes().await.expect("read proxied stream");
    assert_eq!(&body[..], b"chunk-achunk-b");

    server_handle.abort();
    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_rejects_oversized_request_body() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state = test_state_with_openai_base_and_body_limit(
        Url::parse(&upstream_base).expect("valid upstream base url"),
        4,
    )
    .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/echo".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from("hello"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains("request body exceeds")
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_rejects_dot_segment_path() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/%2e%2e/admin".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains(PROXY_DOT_SEGMENT_PATH_NOT_ALLOWED)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_rejects_malformed_percent_encoded_path() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/%zz/models".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains(PROXY_INVALID_REQUEST_TARGET)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_returns_bad_gateway_on_upstream_handshake_timeout() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/hang".parse().expect("valid uri")),
        Method::GET,
        HeaderMap::new(),
        Body::empty(),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn proxy_openai_v1_returns_bad_gateway_on_upstream_handshake_timeout_with_body() {
    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let mut config = test_config();
    config.openai_upstream_base_url = Url::parse(&upstream_base).expect("valid upstream base url");
    config.openai_proxy_handshake_timeout = Duration::from_millis(100);
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    let response = proxy_openai_v1(
        State(state),
        OriginalUri("/v1/hang".parse().expect("valid uri")),
        Method::POST,
        HeaderMap::new(),
        Body::from("hello"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let payload: Value = serde_json::from_slice(&body).expect("decode proxy error payload");
    assert!(
        payload["error"]
            .as_str()
            .expect("error message should be present")
            .contains(PROXY_UPSTREAM_HANDSHAKE_TIMEOUT)
    );

    upstream_handle.abort();
}

#[test]
fn prepare_target_request_body_injects_include_usage_for_chat_stream() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-4o-mini",
        "stream": true,
        "messages": [{"role":"user","content":"hi"}]
    }))
    .expect("serialize request body");
    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::ChatCompletions,
        body,
        true,
        DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
    );
    assert!(did_rewrite);
    assert!(info.is_stream);
    assert_eq!(info.model.as_deref(), Some("gpt-4o-mini"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(
        payload
            .pointer("/stream_options/include_usage")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn prepare_target_request_body_extracts_prompt_cache_key_from_metadata() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "metadata": {
            "prompt_cache_key": "pck-from-body"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::Responses,
        body,
        true,
        DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
    );

    assert_eq!(info.prompt_cache_key.as_deref(), Some("pck-from-body"));
}

#[test]
fn prepare_target_request_body_extracts_requested_service_tier_without_rewriting_when_disabled() {
    let expected = json!({
        "model": "gpt-5.3-codex",
        "serviceTier": " Priority ",
        "stream": false
    });
    let body = serde_json::to_vec(&expected).expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::Responses,
        body,
        true,
        DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
    );

    assert!(!did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode body");
    assert_eq!(payload, expected);
}

#[test]
fn prepare_target_request_body_fill_missing_injects_priority_for_responses() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "input": "hi"
    }))
    .expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::Responses,
        body,
        true,
        ProxyFastModeRewriteMode::FillMissing,
    );

    assert!(did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(payload["service_tier"].as_str(), Some("priority"));
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn prepare_target_request_body_force_priority_overrides_existing_alias() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "messages": [{"role":"user","content":"hi"}],
        "serviceTier": "default"
    }))
    .expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::ChatCompletions,
        body,
        false,
        ProxyFastModeRewriteMode::ForcePriority,
    );

    assert!(did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(payload["service_tier"].as_str(), Some("priority"));
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn prepare_target_request_body_fill_missing_preserves_existing_tier() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "service_tier": "flex"
    }))
    .expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::Responses,
        body,
        true,
        ProxyFastModeRewriteMode::FillMissing,
    );

    assert!(!did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("flex"));
    assert_eq!(
        rewritten,
        serde_json::to_vec(&json!({
            "model": "gpt-5.3-codex",
            "stream": false,
            "service_tier": "flex"
        }))
        .expect("serialize expected body")
    );
}

#[test]
fn prepare_target_request_body_fill_missing_preserves_existing_alias_and_normalizes_field_name() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex"
    }))
    .expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::Responses,
        body,
        true,
        ProxyFastModeRewriteMode::FillMissing,
    );

    assert!(did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("flex"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(payload["service_tier"].as_str(), Some("flex"));
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn prepare_target_request_body_force_priority_overrides_existing_tier() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": false,
        "serviceTier": "flex",
        "messages": [{"role": "user", "content": "hi"}]
    }))
    .expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::ChatCompletions,
        body,
        true,
        ProxyFastModeRewriteMode::ForcePriority,
    );

    assert!(did_rewrite);
    assert_eq!(info.requested_service_tier.as_deref(), Some("priority"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode rewritten body");
    assert_eq!(payload["service_tier"], "priority");
    assert!(payload.get("serviceTier").is_none());
}

#[test]
fn prepare_target_request_body_compact_skips_fast_mode_rewrite() {
    let expected = json!({
        "model": "gpt-5.1-codex-max",
        "serviceTier": "flex",
        "previous_response_id": "resp_prev_001",
        "input": [{
            "role": "user",
            "content": "compact this thread"
        }]
    });
    let body = serde_json::to_vec(&expected).expect("serialize request body");

    let (rewritten, info, did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::ResponsesCompact,
        body,
        true,
        ProxyFastModeRewriteMode::ForcePriority,
    );

    assert!(!did_rewrite);
    assert_eq!(info.model.as_deref(), Some("gpt-5.1-codex-max"));
    assert_eq!(info.requested_service_tier.as_deref(), Some("flex"));
    let payload: Value = serde_json::from_slice(&rewritten).expect("decode body");
    assert_eq!(payload, expected);
    assert!(payload.get("service_tier").is_none());
}

#[test]
fn prepare_target_request_body_extracts_reasoning_effort_for_responses() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "reasoning": {
            "effort": "high"
        }
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::Responses,
        body,
        true,
        DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
    );

    assert_eq!(info.reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn prepare_target_request_body_extracts_reasoning_effort_for_chat_completions() {
    let body = serde_json::to_vec(&json!({
        "model": "gpt-5.3-codex",
        "stream": true,
        "messages": [{"role": "user", "content": "hi"}],
        "reasoning_effort": "medium"
    }))
    .expect("serialize request body");

    let (_rewritten, info, _did_rewrite) = prepare_target_request_body(
        ProxyCaptureTarget::ChatCompletions,
        body,
        true,
        DEFAULT_PROXY_FAST_MODE_REWRITE_MODE,
    );

    assert_eq!(info.reasoning_effort.as_deref(), Some("medium"));
}

#[test]
fn extract_requested_service_tier_from_request_body_reads_top_level_aliases() {
    let snake_case = json!({ "service_tier": " Priority " });
    let camel_case = json!({ "serviceTier": "PRIORITY" });

    assert_eq!(
        extract_requested_service_tier_from_request_body(&snake_case).as_deref(),
        Some("priority")
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&camel_case).as_deref(),
        Some("priority")
    );
}

#[test]
fn extract_requested_service_tier_from_request_body_ignores_nested_or_non_string_values() {
    let nested = json!({
        "response": { "service_tier": "priority" },
        "metadata": { "serviceTier": "priority" }
    });
    let non_string = json!({ "service_tier": true });
    let blank = json!({ "serviceTier": "   " });

    assert_eq!(
        extract_requested_service_tier_from_request_body(&nested),
        None
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&non_string),
        None
    );
    assert_eq!(
        extract_requested_service_tier_from_request_body(&blank),
        None
    );
}

#[test]
fn extract_requester_ip_uses_expected_header_priority() {
    let mut preferred = HeaderMap::new();
    preferred.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.10, 203.0.113.9"),
    );
    preferred.insert(
        HeaderName::from_static("x-real-ip"),
        HeaderValue::from_static("203.0.113.5"),
    );
    preferred.insert(
        HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=192.0.2.60;proto=https"),
    );
    assert_eq!(
        extract_requester_ip(&preferred, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("198.51.100.10")
    );

    let mut fallback_forwarded = HeaderMap::new();
    fallback_forwarded.insert(
        HeaderName::from_static("forwarded"),
        HeaderValue::from_static("for=\"[2001:db8::1]:443\";proto=https"),
    );
    assert_eq!(
        extract_requester_ip(&fallback_forwarded, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("2001:db8::1")
    );

    let no_headers = HeaderMap::new();
    assert_eq!(
        extract_requester_ip(&no_headers, Some(IpAddr::from([127, 0, 0, 1]))).as_deref(),
        Some("127.0.0.1")
    );
}

#[test]
fn extract_prompt_cache_key_from_headers_reads_whitelist_keys() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-prompt-cache-key"),
        HeaderValue::from_static("pck-from-header"),
    );
    assert_eq!(
        extract_prompt_cache_key_from_headers(&headers).as_deref(),
        Some("pck-from-header")
    );
}

#[test]
fn parse_stream_response_payload_extracts_usage_and_model() {
    let raw = [
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}],\"usage\":null}",
        "data: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o-mini\",\"choices\":[],\"service_tier\":\"priority\",\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7,\"total_tokens\":18}}",
        "data: [DONE]",
    ]
    .join("\n");
    let parsed = parse_stream_response_payload(raw.as_bytes());
    assert_eq!(parsed.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(parsed.usage.input_tokens, Some(11));
    assert_eq!(parsed.usage.output_tokens, Some(7));
    assert_eq!(parsed.usage.total_tokens, Some(18));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn estimate_proxy_cost_subtracts_cached_tokens_from_base_input_rate() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: Some(0.5),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, price_version) = estimate_proxy_cost(&catalog, Some("gpt-test"), &usage);

    let expected = ((600.0 * 1.0) + (200.0 * 2.0) + (400.0 * 0.5)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
    assert_eq!(price_version.as_deref(), Some("unit-test"));
}

#[test]
fn estimate_proxy_cost_keeps_full_input_when_cache_price_missing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-test".to_string(),
            ModelPricing {
                input_per_1m: 1.0,
                output_per_1m: 2.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1_000),
        output_tokens: Some(200),
        cache_input_tokens: Some(400),
        reasoning_tokens: None,
        total_tokens: Some(1_200),
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-test"), &usage);

    let expected = ((1_000.0 * 1.0) + (200.0 * 2.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_falls_back_to_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(500),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(1500),
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.2-2025-12-11"), &usage);

    let expected = ((1000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_prefers_exact_model_over_dated_model_base_pricing() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([
            (
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 1.0,
                    output_per_1m: 1.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
            (
                "gpt-5.2-2025-12-11".to_string(),
                ModelPricing {
                    input_per_1m: 4.0,
                    output_per_1m: 5.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
        ]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(1000),
        output_tokens: Some(1000),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: Some(2000),
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.2-2025-12-11"), &usage);

    let expected = ((1000.0 * 4.0) + (1000.0 * 5.0)) / 1_000_000.0;
    assert!((cost.expect("cost should be present") - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_at_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4"), &usage);

    let expected = ((271_000.0 * 2.5) + (1_000.0 * 0.25) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4"), &usage);

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_to_reasoning_cost() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: Some(20.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4"), &usage);

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 20.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_above_threshold() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4-pro"), &usage);

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_pro_long_context_surcharge_for_dated_model_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4-pro".to_string(),
            ModelPricing {
                input_per_1m: 30.0,
                output_per_1m: 180.0,
                cache_input_per_1m: None,
                reasoning_per_1m: Some(90.0),
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(999_999),
        reasoning_tokens: Some(2_000),
        total_tokens: None,
    };

    let (cost, estimated, _) =
        estimate_proxy_cost(&catalog, Some("gpt-5.4-pro-2026-03-01"), &usage);

    let input_part = (272_001.0 * 30.0) / 1_000_000.0;
    let output_part = (1_000.0 * 180.0) / 1_000_000.0;
    let reasoning_part = (2_000.0 * 90.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5) + (reasoning_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_applies_gpt_5_4_long_context_surcharge_for_dated_model_suffix() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: Some(0.25),
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: Some(1_000),
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4-2026-03-01"), &usage);

    let input_part = ((271_001.0 * 2.5) + (1_000.0 * 0.25)) / 1_000_000.0;
    let output_part = (1_000.0 * 15.0) / 1_000_000.0;
    let expected = (input_part * 2.0) + (output_part * 1.5);
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn estimate_proxy_cost_does_not_apply_gpt_5_4_long_context_surcharge_for_other_models() {
    let catalog = PricingCatalog {
        version: "unit-test".to_string(),
        models: HashMap::from([(
            "gpt-5.4o".to_string(),
            ModelPricing {
                input_per_1m: 2.5,
                output_per_1m: 15.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };
    let usage = ParsedUsage {
        input_tokens: Some(GPT_5_4_LONG_CONTEXT_THRESHOLD_TOKENS + 1),
        output_tokens: Some(1_000),
        cache_input_tokens: None,
        reasoning_tokens: None,
        total_tokens: None,
    };

    let (cost, estimated, _) = estimate_proxy_cost(&catalog, Some("gpt-5.4o"), &usage);

    let expected = ((272_001.0 * 2.5) + (1_000.0 * 15.0)) / 1_000_000.0;
    let computed = cost.expect("cost should be present");
    assert!((computed - expected).abs() < 1e-12);
    assert!(estimated);
}

#[test]
fn parse_target_response_payload_decodes_gzip_stream_usage() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"priority\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_decodes_multi_value_content_encoding() {
    let raw = [
        "event: response.created",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"in_progress\"}}",
        "",
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"service_tier\":\"flex\",\"usage\":{\"input_tokens\":12,\"output_tokens\":3,\"total_tokens\":15,\"input_tokens_details\":{\"cached_tokens\":2}}}}",
        "",
    ]
    .join("\n");

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        &compressed,
        true,
        Some("identity, gzip"),
    );
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.input_tokens, Some(12));
    assert_eq!(parsed.usage.output_tokens, Some(3));
    assert_eq!(parsed.usage.cache_input_tokens, Some(2));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert_eq!(parsed.service_tier.as_deref(), Some("flex"));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_detects_sse_without_request_stream_hint() {
    let raw = [
        "event: response.completed",
        r#"data: {"type":"response.completed","response":{"model":"gpt-5.3-codex","service_tier":"priority","usage":{"input_tokens":12,"output_tokens":3,"total_tokens":15}}}"#,
        "",
    ]
    .join("\n");

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, raw.as_bytes(), false, None);

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(15));
    assert!(parsed.usage_missing_reason.is_none());
}

#[test]
fn parse_target_response_payload_reads_service_tier_from_response_object() {
    let raw = json!({
        "id": "resp_json_1",
        "response": {
            "model": "gpt-5.3-codex",
            "service_tier": "priority",
            "usage": {
                "input_tokens": 21,
                "output_tokens": 5,
                "total_tokens": 26
            }
        }
    });

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        serde_json::to_string(&raw)
            .expect("serialize raw payload")
            .as_bytes(),
        false,
        None,
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.service_tier.as_deref(), Some("priority"));
    assert_eq!(parsed.usage.total_tokens, Some(26));
}

#[test]
fn parse_target_response_payload_records_decode_failure_reason() {
    let raw = [
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_test\",\"model\":\"gpt-5.3-codex\",\"status\":\"completed\",\"usage\":{\"input_tokens\":10,\"output_tokens\":2,\"total_tokens\":12}}}",
        "data: [DONE]",
    ]
    .join("\n");

    let parsed = parse_target_response_payload(
        ProxyCaptureTarget::Responses,
        raw.as_bytes(),
        true,
        Some("gzip"),
    );

    assert_eq!(parsed.model.as_deref(), Some("gpt-5.3-codex"));
    assert_eq!(parsed.usage.total_tokens, Some(12));
    assert!(
        parsed
            .usage_missing_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("response_decode_failed:gzip:"))
    );
}

#[tokio::test]
async fn proxy_capture_target_extracts_usage_from_gzip_response_stream() {
    #[derive(sqlx::FromRow)]
    struct PersistedUsageRow {
        source: String,
        status: Option<String>,
        input_tokens: Option<i64>,
        output_tokens: Option<i64>,
        cache_input_tokens: Option<i64>,
        total_tokens: Option<i64>,
        payload: Option<String>,
    }

    let (upstream_base, upstream_handle) = spawn_test_upstream().await;
    let state =
        test_state_with_openai_base(Url::parse(&upstream_base).expect("valid upstream base url"))
            .await;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-forwarded-for"),
        HeaderValue::from_static("198.51.100.42, 203.0.113.10"),
    );
    let response = proxy_openai_v1(
        State(state.clone()),
        OriginalUri("/v1/responses?mode=gzip".parse().expect("valid uri")),
        Method::POST,
        headers,
        Body::from(
            r#"{"model":"gpt-5.3-codex","stream":true,"metadata":{"prompt_cache_key":"pck-gzip-1"},"input":"hello"}"#,
        ),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    let _ = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");

    let mut row: Option<PersistedUsageRow> = None;
    for _ in 0..50 {
        row = sqlx::query_as::<_, PersistedUsageRow>(
            r#"
            SELECT source, status, input_tokens, output_tokens, cache_input_tokens, total_tokens, payload
            FROM codex_invocations
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&state.pool)
        .await
        .expect("query capture record");

        if row
            .as_ref()
            .is_some_and(|record| record.input_tokens.is_some())
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let row = row.expect("capture record should exist");
    assert_eq!(row.source, SOURCE_PROXY);
    assert_eq!(row.status.as_deref(), Some("success"));
    assert_eq!(row.input_tokens, Some(12));
    assert_eq!(row.output_tokens, Some(3));
    assert_eq!(row.cache_input_tokens, Some(2));
    assert_eq!(row.total_tokens, Some(15));

    let payload: Value = serde_json::from_str(row.payload.as_deref().unwrap_or("{}"))
        .expect("decode payload summary");
    assert_eq!(payload["endpoint"], "/v1/responses");
    assert!(payload["usageMissingReason"].is_null());
    assert_eq!(payload["requesterIp"], "198.51.100.42");
    assert_eq!(payload["promptCacheKey"], "pck-gzip-1");
    assert!(
        payload["proxyWeightDelta"].is_number(),
        "proxy weight delta should be recorded for fresh proxy attempts"
    );

    upstream_handle.abort();
}

#[tokio::test]
async fn resolve_default_source_scope_always_all() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let scope_before = resolve_default_source_scope(&pool)
        .await
        .expect("scope before insert");
    assert_eq!(scope_before, InvocationSourceScope::All);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, raw_response
        )
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind("proxy-test-1")
    .bind("2026-02-22 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert proxy invocation");

    let scope_after = resolve_default_source_scope(&pool)
        .await
        .expect("scope after insert");
    assert_eq!(scope_after, InvocationSourceScope::All);
}

#[tokio::test]
async fn list_invocations_projects_payload_context_fields() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-context-1")
    .bind("2026-02-25 10:00:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind(
        r#"{"endpoint":"/v1/responses","failureKind":"upstream_stream_error","requesterIp":"198.51.100.77","promptCacheKey":"pck-list-1","requestedServiceTier":"priority","serviceTier":null,"service_tier":"priority","proxyDisplayName":"jp-relay-01","proxyWeightDelta":-0.68,"reasoningEffort":"high"}"#,
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert proxy invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
        }),
    )
    .await
    .expect("list invocations should succeed");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-context-1")
        .expect("inserted invocation should be present");
    assert_eq!(record.endpoint.as_deref(), Some("/v1/responses"));
    assert_eq!(
        record.failure_kind.as_deref(),
        Some("upstream_stream_error")
    );
    assert_eq!(record.requester_ip.as_deref(), Some("198.51.100.77"));
    assert_eq!(record.prompt_cache_key.as_deref(), Some("pck-list-1"));
    assert_eq!(record.requested_service_tier.as_deref(), Some("priority"));
    assert_eq!(record.service_tier.as_deref(), Some("priority"));
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-01"));
    assert_eq!(record.proxy_weight_delta, Some(-0.68));
    assert_eq!(record.reasoning_effort.as_deref(), Some("high"));
}

#[tokio::test]
async fn list_invocations_tolerates_malformed_payload_json() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-context-malformed")
    .bind("2026-02-25 10:01:00")
    .bind(SOURCE_PROXY)
    .bind("failed")
    .bind("not-json")
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert malformed payload invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
        }),
    )
    .await
    .expect("list invocations should tolerate malformed payload");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-context-malformed")
        .expect("inserted invocation should be present");
    assert_eq!(record.endpoint, None);
    assert_eq!(record.failure_kind, None);
    assert_eq!(record.requester_ip, None);
    assert_eq!(record.prompt_cache_key, None);
    assert_eq!(record.requested_service_tier, None);
    assert_eq!(record.service_tier, None);
    assert_eq!(record.proxy_weight_delta, None);
    assert_eq!(record.reasoning_effort, None);
}

#[tokio::test]
async fn list_invocations_ignores_non_numeric_proxy_weight_delta() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("proxy-context-delta-text")
    .bind("2026-02-25 10:02:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(
        "{\"endpoint\":\"/v1/responses\",\"proxyDisplayName\":\"jp-relay-02\",\"proxyWeightDelta\":\"abc\"}",
    )
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert non-numeric proxyWeightDelta invocation");

    let Json(response) = list_invocations(
        State(state),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
        }),
    )
    .await
    .expect("list invocations should ignore non-numeric proxyWeightDelta");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "proxy-context-delta-text")
        .expect("inserted invocation should be present");
    assert_eq!(record.proxy_display_name.as_deref(), Some("jp-relay-02"));
    assert_eq!(record.proxy_weight_delta, None);
}

#[tokio::test]
async fn list_invocations_preserves_historical_xy_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            model,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind("xy-history-1")
    .bind("2026-02-25 10:03:00")
    .bind(SOURCE_XY)
    .bind("gpt-5.3-codex")
    .bind(16_i64)
    .bind(0.0042_f64)
    .bind("success")
    .bind(r#"{"serviceTier":"priority"}"#)
    .bind(r#"{"legacy":true}"#)
    .execute(&state.pool)
    .await
    .expect("insert historical xy invocation");

    let Json(response) = list_invocations(
        State(state.clone()),
        Query(ListQuery {
            limit: Some(10),
            model: None,
            status: None,
        }),
    )
    .await
    .expect("list invocations should keep historical xy rows");

    let record = response
        .records
        .into_iter()
        .find(|item| item.invoke_id == "xy-history-1")
        .expect("historical xy row should be returned");
    assert_eq!(record.source, SOURCE_XY);
    assert_eq!(record.service_tier.as_deref(), Some("priority"));
    assert_eq!(record.requested_service_tier, None);
}

#[tokio::test]
async fn stats_endpoints_preserve_historical_xy_records() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            total_tokens,
            cost,
            status,
            payload,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("xy-history-stats-1")
    .bind(&occurred_at)
    .bind(SOURCE_XY)
    .bind(16_i64)
    .bind(0.0042_f64)
    .bind("success")
    .bind(r#"{"serviceTier":"priority"}"#)
    .bind(r#"{"legacy":true}"#)
    .execute(&state.pool)
    .await
    .expect("insert historical xy stats row");

    let Json(stats) = fetch_stats(State(state.clone()))
        .await
        .expect("fetch_stats should include historical xy rows");
    assert_eq!(stats.total_count, 1);
    assert_eq!(stats.success_count, 1);
    assert_eq!(stats.failure_count, 0);
    assert_eq!(stats.total_tokens, 16);
    assert_f64_close(stats.total_cost, 0.0042);

    let Json(summary) = fetch_summary(
        State(state.clone()),
        Query(SummaryQuery {
            window: Some("1d".to_string()),
            limit: None,
            time_zone: None,
        }),
    )
    .await
    .expect("fetch_summary should include historical xy rows");
    assert_eq!(summary.total_count, 1);
    assert_eq!(summary.success_count, 1);
    assert_eq!(summary.failure_count, 0);
    assert_eq!(summary.total_tokens, 16);
    assert_f64_close(summary.total_cost, 0.0042);

    let Json(timeseries) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1h".to_string()),
            settlement_hour: None,
            time_zone: None,
        }),
    )
    .await
    .expect("fetch_timeseries should include historical xy rows");
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_count)
            .sum::<i64>(),
        1
    );
    assert_eq!(
        timeseries
            .points
            .iter()
            .map(|point| point.total_tokens)
            .sum::<i64>(),
        16
    );
    assert_f64_close(
        timeseries
            .points
            .iter()
            .map(|point| point.total_cost)
            .sum::<f64>(),
        0.0042,
    );
}

#[test]
fn normalize_prompt_cache_conversation_limit_accepts_whitelist_values_only() {
    assert_eq!(normalize_prompt_cache_conversation_limit(None), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(20)), 20);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(50)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(100)), 100);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(10)), 50);
    assert_eq!(normalize_prompt_cache_conversation_limit(Some(200)), 50);
}

#[tokio::test]
async fn prompt_cache_conversations_groups_recent_keys_and_uses_history_totals() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();

    async fn insert_row(
        pool: &Pool<Sqlite>,
        invoke_id: &str,
        occurred_at: DateTime<Utc>,
        key: Option<&str>,
        status: &str,
        total_tokens: i64,
        cost: f64,
    ) {
        let payload = match key {
            Some(key) => json!({ "promptCacheKey": key }).to_string(),
            None => "{}".to_string(),
        };
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
        )
        .bind(invoke_id)
        .bind(format_naive(
            occurred_at.with_timezone(&Shanghai).naive_local(),
        ))
        .bind(SOURCE_PROXY)
        .bind(status)
        .bind(total_tokens)
        .bind(cost)
        .bind(payload)
        .bind("{}")
        .execute(pool)
        .await
        .expect("insert invocation row");
    }

    // key-a: active in 24h + older history
    insert_row(
        &state.pool,
        "pck-a-history",
        now - ChronoDuration::hours(48),
        Some("pck-a"),
        "success",
        100,
        1.0,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-a-24h-1",
        now - ChronoDuration::hours(2),
        Some("pck-a"),
        "success",
        20,
        0.2,
    )
    .await;
    insert_row(
        &state.pool,
        "pck-a-24h-2",
        now - ChronoDuration::hours(1),
        Some("pck-a"),
        "failed",
        30,
        0.3,
    )
    .await;

    // key-b: newer created_at so it should rank before key-a.
    insert_row(
        &state.pool,
        "pck-b-24h-1",
        now - ChronoDuration::hours(10),
        Some("pck-b"),
        "success",
        10,
        0.1,
    )
    .await;

    // key-c: not active in last 24h; should be excluded.
    insert_row(
        &state.pool,
        "pck-c-history",
        now - ChronoDuration::hours(72),
        Some("pck-c"),
        "success",
        8,
        0.08,
    )
    .await;

    // missing key in last 24h; should be ignored.
    insert_row(
        &state.pool,
        "pck-missing-24h",
        now - ChronoDuration::minutes(40),
        None,
        "success",
        999,
        9.99,
    )
    .await;

    let Json(response) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery { limit: Some(20) }),
    )
    .await
    .expect("prompt cache conversation stats should succeed");

    assert_eq!(response.conversations.len(), 2);
    assert_eq!(response.conversations[0].prompt_cache_key, "pck-b");
    assert_eq!(response.conversations[1].prompt_cache_key, "pck-a");

    let key_a = response
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-a")
        .expect("pck-a should be included");
    assert_eq!(key_a.request_count, 3);
    assert_eq!(key_a.total_tokens, 150);
    assert!((key_a.total_cost - 1.5).abs() < 1e-9);
    assert_eq!(key_a.last24h_requests.len(), 2);
    assert_eq!(key_a.last24h_requests[0].request_tokens, 20);
    assert_eq!(key_a.last24h_requests[0].cumulative_tokens, 20);
    assert!(key_a.last24h_requests[0].is_success);
    assert_eq!(key_a.last24h_requests[1].request_tokens, 30);
    assert_eq!(key_a.last24h_requests[1].cumulative_tokens, 50);
    assert!(!key_a.last24h_requests[1].is_success);
}

#[tokio::test]
async fn prompt_cache_conversations_cache_reuses_recent_result_within_ttl() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred_a = format_naive(
        (now - ChronoDuration::minutes(80))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    let occurred_b = format_naive(
        (now - ChronoDuration::minutes(30))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-cache-1")
    .bind(&occurred_a)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(10)
    .bind(0.01)
    .bind(r#"{"promptCacheKey":"pck-cache"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert first cache row");

    let Json(first) = fetch_prompt_cache_conversations(
        State(state.clone()),
        Query(PromptCacheConversationsQuery { limit: Some(20) }),
    )
    .await
    .expect("first fetch should succeed");
    let first_count = first
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-cache")
        .map(|item| item.request_count)
        .expect("pck-cache should be present");
    assert_eq!(first_count, 1);

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-cache-2")
    .bind(&occurred_b)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(15)
    .bind(0.015)
    .bind(r#"{"promptCacheKey":"pck-cache"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert second cache row");

    let Json(second) = fetch_prompt_cache_conversations(
        State(state),
        Query(PromptCacheConversationsQuery { limit: Some(20) }),
    )
    .await
    .expect("second fetch should use cached result");
    let second_count = second
        .conversations
        .iter()
        .find(|item| item.prompt_cache_key == "pck-cache")
        .map(|item| item.request_count)
        .expect("pck-cache should still be present");
    assert_eq!(second_count, 1);
}

#[tokio::test]
async fn prompt_cache_conversations_concurrent_requests_same_limit_do_not_stall() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let now = Utc::now();
    let occurred = format_naive(
        (now - ChronoDuration::minutes(20))
            .with_timezone(&Shanghai)
            .naive_local(),
    );

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, cost, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind("pck-concurrent-1")
    .bind(&occurred)
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(18)
    .bind(0.018)
    .bind(r#"{"promptCacheKey":"pck-concurrent"}"#)
    .bind("{}")
    .execute(&state.pool)
    .await
    .expect("insert concurrent cache row");

    let mut handles = Vec::new();
    for _ in 0..8 {
        let state_clone = state.clone();
        handles.push(tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_secs(2),
                fetch_prompt_cache_conversations(
                    State(state_clone),
                    Query(PromptCacheConversationsQuery { limit: Some(20) }),
                ),
            )
            .await
        }));
    }

    for handle in handles {
        let response = handle
            .await
            .expect("join should succeed")
            .expect("concurrent request should not timeout")
            .expect("concurrent request should succeed");
        let Json(payload) = response;
        assert!(
            payload
                .conversations
                .iter()
                .any(|item| item.prompt_cache_key == "pck-concurrent"),
            "expected pck-concurrent to be present in each response",
        );
    }
}

#[tokio::test]
async fn prompt_cache_conversation_flight_guard_cleans_in_flight_on_drop() {
    let cache = Arc::new(Mutex::new(PromptCacheConversationsCacheState::default()));
    let (signal, _receiver) = watch::channel(false);
    {
        let mut state = cache.lock().await;
        state
            .in_flight
            .insert(20, PromptCacheConversationInFlight { signal });
    }

    {
        let _guard = PromptCacheConversationFlightGuard::new(cache.clone(), 20);
    }

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let has_entry = {
                let state = cache.lock().await;
                state.in_flight.contains_key(&20)
            };
            if !has_entry {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("drop cleanup should remove in-flight marker");
}

#[test]
fn decode_response_payload_for_usage_decompresses_gzip_stream() {
    let raw = [
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":123,\"output_tokens\":45,\"total_tokens\":168,\"input_tokens_details\":{\"cached_tokens\":7},\"output_tokens_details\":{\"reasoning_tokens\":4}}}}",
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");

    let (decoded, decode_error) = decode_response_payload_for_usage(&compressed, Some("gzip"));
    assert!(decode_error.is_none());

    let parsed =
        parse_target_response_payload(ProxyCaptureTarget::Responses, decoded.as_ref(), true, None);
    assert_eq!(parsed.usage.input_tokens, Some(123));
    assert_eq!(parsed.usage.output_tokens, Some(45));
    assert_eq!(parsed.usage.total_tokens, Some(168));
    assert_eq!(parsed.usage.cache_input_tokens, Some(7));
    assert_eq!(parsed.usage.reasoning_tokens, Some(4));
}

#[tokio::test]
async fn backfill_proxy_prompt_cache_keys_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload(&request_path, Some("pck-backfill-1"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-1",
        &request_path,
        "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\",\"codexSessionId\":\"legacy-session-1\"}",
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-ready",
        &request_path,
        "{\"endpoint\":\"/v1/responses\",\"promptCacheKey\":\"already-present\"}",
    )
    .await;

    let summary_first = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("first prompt cache key backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_key, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-backfill-1");
    assert!(
        payload_json.get("codexSessionId").is_none(),
        "legacy codexSessionId key should be removed during backfill"
    );

    let summary_second = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("second prompt cache key backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_prompt_cache_keys_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-skips");
    let ok_request_path = temp_dir.join("request-ok.json");
    let missing_key_request_path = temp_dir.join("request-missing-key.json");
    let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
    let missing_file_request_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload(&ok_request_path, Some("pck-backfill-ok"));
    write_backfill_request_payload(&missing_key_request_path, None);
    fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

    let base_payload = "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}";
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-ok",
        &ok_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-missing-file",
        &missing_file_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-invalid-json",
        &invalid_json_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-missing-key",
        &missing_key_request_path,
        base_payload,
    )
    .await;

    let summary = backfill_proxy_prompt_cache_keys(&pool, None)
        .await
        .expect("prompt cache key backfill should succeed");
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_key, 1);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-ok")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-backfill-ok");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_requested_service_tiers_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload_with_requested_service_tier(
        &request_path,
        Some("priority"),
        ProxyCaptureTarget::Responses,
    );

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-backfill-1",
        &request_path,
        r#"{"endpoint":"/v1/responses"}"#,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-backfill-ready",
        &request_path,
        r#"{"endpoint":"/v1/responses","requestedServiceTier":"priority"}"#,
    )
    .await;

    let summary_first = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("first requested service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-requested-tier-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["requestedServiceTier"], "priority");

    let summary_second = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("second requested service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_requested_service_tiers_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-requested-service-tier-backfill-skips");
    let missing_tier_request_path = temp_dir.join("request-missing-tier.json");
    let invalid_json_request_path = temp_dir.join("request-invalid-json.json");
    let missing_file_request_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload_with_requested_service_tier(
        &missing_tier_request_path,
        None,
        ProxyCaptureTarget::Responses,
    );
    fs::write(&invalid_json_request_path, b"not-json").expect("write invalid request payload");

    let base_payload = r#"{"endpoint":"/v1/responses"}"#;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-missing-file",
        &missing_file_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-invalid-json",
        &invalid_json_request_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-requested-tier-missing-tier",
        &missing_tier_request_path,
        base_payload,
    )
    .await;

    let summary = backfill_proxy_requested_service_tiers(&pool, None)
        .await
        .expect("requested service tier backfill should succeed");
    assert_eq!(summary.scanned, 3);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_tier, 1);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_reasoning_efforts_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill");
    let request_path = temp_dir.join("request.json");
    write_backfill_request_payload_with_reasoning(
        &request_path,
        Some("pck-reasoning"),
        Some("high"),
        ProxyCaptureTarget::Responses,
    );

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-backfill-1",
        &request_path,
        r#"{"endpoint":"/v1/responses","requesterIp":"198.51.100.77"}"#,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-backfill-ready",
        &request_path,
        r#"{"endpoint":"/v1/responses","reasoningEffort":"medium"}"#,
    )
    .await;

    let summary_first = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("first reasoning effort backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_invalid_json, 0);
    assert_eq!(summary_first.skipped_missing_effort, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-reasoning-backfill-1")
            .fetch_one(&pool)
            .await
            .expect("query reasoning backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["reasoningEffort"], "high");

    let summary_second = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("second reasoning effort backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_reasoning_efforts_tracks_skip_counters_and_chat_payloads() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-reasoning-effort-backfill-skips");
    let ok_chat_path = temp_dir.join("request-chat-ok.json");
    let missing_effort_path = temp_dir.join("request-missing-effort.json");
    let invalid_json_path = temp_dir.join("request-invalid-json.json");
    let missing_file_path = temp_dir.join("request-missing.json");

    write_backfill_request_payload_with_reasoning(
        &ok_chat_path,
        None,
        Some("medium"),
        ProxyCaptureTarget::ChatCompletions,
    );
    write_backfill_request_payload_with_reasoning(
        &missing_effort_path,
        None,
        None,
        ProxyCaptureTarget::Responses,
    );
    fs::write(&invalid_json_path, b"not-json").expect("write invalid request payload");

    let base_payload = r#"{"endpoint":"/v1/chat/completions"}"#;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-chat-ok",
        &ok_chat_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-missing-file",
        &missing_file_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-invalid-json",
        &invalid_json_path,
        base_payload,
    )
    .await;
    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-reasoning-missing-effort",
        &missing_effort_path,
        r#"{"endpoint":"/v1/responses"}"#,
    )
    .await;

    let summary = backfill_proxy_reasoning_efforts(&pool, None)
        .await
        .expect("reasoning effort backfill should succeed");
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_invalid_json, 1);
    assert_eq!(summary.skipped_missing_effort, 1);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-reasoning-chat-ok")
            .fetch_one(&pool)
            .await
            .expect("query chat reasoning payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["reasoningEffort"], "medium");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_prompt_cache_keys_reads_from_fallback_root_for_relative_paths() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-prompt-cache-key-backfill-fallback");
    let fallback_root = temp_dir.join("legacy-root");
    let relative_path = PathBuf::from("proxy_raw_payloads/request-fallback.json");
    let request_path = fallback_root.join(&relative_path);
    let request_dir = request_path.parent().expect("request parent dir");
    fs::create_dir_all(request_dir).expect("create fallback request dir");
    write_backfill_request_payload(&request_path, Some("pck-fallback-1"));

    insert_proxy_prompt_cache_backfill_row(
        &pool,
        "proxy-pck-backfill-fallback",
        &relative_path,
        "{\"endpoint\":\"/v1/responses\",\"requesterIp\":\"198.51.100.77\"}",
    )
    .await;

    let summary = backfill_proxy_prompt_cache_keys(&pool, Some(&fallback_root))
        .await
        .expect("prompt cache key backfill with fallback root should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 0);
    assert_eq!(summary.skipped_invalid_json, 0);
    assert_eq!(summary.skipped_missing_key, 0);

    let payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-pck-backfill-fallback")
            .fetch_one(&pool)
            .await
            .expect("query fallback-backfilled payload");
    let payload_json: Value = serde_json::from_str(&payload).expect("decode payload JSON");
    assert_eq!(payload_json["promptCacheKey"], "pck-fallback-1");

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_updates_payload_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("invocation-service-tier-backfill");
    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload_with_service_tier(&response_path, Some("priority"));

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("quota-service-tier-backfill")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_XY)
    .bind("success")
    .bind("{}")
    .bind(r#"{"service_tier":"priority"}"#)
    .execute(&pool)
    .await
    .expect("insert quota service tier row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("proxy-service-tier-backfill")
    .bind("2026-02-23 00:00:01")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind("{}")
    .bind(response_path.to_string_lossy().to_string())
    .execute(&pool)
    .await
    .expect("insert proxy service tier row");

    let summary_first = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("first service tier backfill should succeed");
    assert_eq!(summary_first.scanned, 2);
    assert_eq!(summary_first.updated, 2);
    assert_eq!(summary_first.skipped_missing_file, 0);
    assert_eq!(summary_first.skipped_missing_tier, 0);

    let quota_payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("quota-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query quota payload");
    let quota_payload_json: Value =
        serde_json::from_str(&quota_payload).expect("decode quota payload JSON");
    assert_eq!(quota_payload_json["serviceTier"], "priority");

    let proxy_payload: String =
        sqlx::query_scalar("SELECT payload FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-service-tier-backfill")
            .fetch_one(&pool)
            .await
            .expect("query proxy payload");
    let proxy_payload_json: Value =
        serde_json::from_str(&proxy_payload).expect("decode proxy payload JSON");
    assert_eq!(proxy_payload_json["serviceTier"], "priority");

    let summary_second = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("second service tier backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_invocation_service_tiers_tracks_skip_counters() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
    )
    .bind("service-tier-missing")
    .bind("2026-02-23 00:00:00")
    .bind(SOURCE_XY)
    .bind("success")
    .bind("{}")
    .bind(r#"{"status":"success"}"#)
    .execute(&pool)
    .await
    .expect("insert missing tier row");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("service-tier-missing-file")
    .bind("2026-02-23 00:00:01")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(r#"{"endpoint":"/v1/responses"}"#)
    .bind("{}")
    .bind("/tmp/does-not-exist-response.bin")
    .execute(&pool)
    .await
    .expect("insert missing file row");

    let summary = backfill_invocation_service_tiers(&pool, None)
        .await
        .expect("service tier backfill skip run should succeed");
    assert_eq!(summary.scanned, 2);
    assert_eq!(summary.updated, 0);
    assert_eq!(summary.skipped_missing_file, 1);
    assert_eq!(summary.skipped_missing_tier, 1);
}

#[tokio::test]
async fn backfill_proxy_usage_tokens_updates_missing_tokens_idempotently() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = std::env::temp_dir().join(format!(
        "proxy-usage-backfill-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let response_path = temp_dir.join("response.bin");
    let raw = [
        "event: response.completed",
        "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":88,\"output_tokens\":22,\"total_tokens\":110,\"input_tokens_details\":{\"cached_tokens\":9},\"output_tokens_details\":{\"reasoning_tokens\":3}}}}",
    ]
    .join("\n");
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(raw.as_bytes())
        .expect("write gzip payload");
    let compressed = encoder.finish().expect("finish gzip payload");
    fs::write(&response_path, compressed).expect("write response payload");

    let row_count = BACKFILL_BATCH_SIZE as usize + 5;
    for index in 0..row_count {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id, occurred_at, source, status, payload, raw_response, response_raw_path
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(format!("proxy-backfill-test-{index}"))
        .bind("2026-02-23 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("success")
        .bind(
            "{\"endpoint\":\"/v1/responses\",\"statusCode\":200,\"isStream\":true,\"requestModel\":null,\"responseModel\":null,\"usageMissingReason\":null,\"requestParseError\":null}",
        )
        .bind("{}")
        .bind(response_path.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("insert proxy row");
    }

    let summary_first = backfill_proxy_usage_tokens(&pool, None)
        .await
        .expect("first backfill should succeed");
    assert_eq!(summary_first.scanned, row_count as u64);
    assert_eq!(summary_first.updated, row_count as u64);

    let row = sqlx::query(
        r#"
        SELECT
          COUNT(*) AS total_rows,
          SUM(CASE WHEN input_tokens = 88 THEN 1 ELSE 0 END) AS input_tokens_88,
          SUM(CASE WHEN output_tokens = 22 THEN 1 ELSE 0 END) AS output_tokens_22,
          SUM(CASE WHEN cache_input_tokens = 9 THEN 1 ELSE 0 END) AS cache_input_tokens_9,
          SUM(CASE WHEN reasoning_tokens = 3 THEN 1 ELSE 0 END) AS reasoning_tokens_3,
          SUM(CASE WHEN total_tokens = 110 THEN 1 ELSE 0 END) AS total_tokens_110
        FROM codex_invocations
        WHERE source = ?1
        "#,
    )
    .bind(SOURCE_PROXY)
    .fetch_one(&pool)
    .await
    .expect("fetch backfilled rows");
    assert_eq!(
        row.try_get::<i64, _>("total_rows")
            .expect("read total_rows"),
        row_count as i64
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("input_tokens_88")
            .expect("read input_tokens_88"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("output_tokens_22")
            .expect("read output_tokens_22"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cache_input_tokens_9")
            .expect("read cache_input_tokens_9"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("reasoning_tokens_3")
            .expect("read reasoning_tokens_3"),
        Some(row_count as i64)
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("total_tokens_110")
            .expect("read total_tokens_110"),
        Some(row_count as i64)
    );

    let summary_second = backfill_proxy_usage_tokens(&pool, None)
        .await
        .expect("second backfill should succeed");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_usage_tokens_reads_from_fallback_root_for_relative_paths() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-usage-backfill-fallback");
    let fallback_root = temp_dir.join("legacy-root");
    let relative_path = PathBuf::from("proxy_raw_payloads/response-fallback.bin");
    let response_path = fallback_root.join(&relative_path);
    let response_dir = response_path.parent().expect("response parent dir");
    fs::create_dir_all(response_dir).expect("create fallback response dir");
    write_backfill_response_payload(&response_path);

    insert_proxy_backfill_row(&pool, "proxy-usage-backfill-fallback", &relative_path).await;
    let row_id: i64 = sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
        .bind("proxy-usage-backfill-fallback")
        .fetch_one(&pool)
        .await
        .expect("query fallback row id");

    let summary = backfill_proxy_usage_tokens_up_to_id(&pool, row_id, Some(&fallback_root))
        .await
        .expect("usage backfill with fallback root should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_missing_file, 0);
    assert_eq!(summary.skipped_without_usage, 0);
    assert_eq!(summary.skipped_decode_error, 0);

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-usage-backfill-fallback")
            .fetch_one(&pool)
            .await
            .expect("query fallback usage row");
    assert_eq!(total_tokens, Some(110));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_usage_tokens_respects_snapshot_upper_bound() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let temp_dir = make_temp_test_dir("proxy-usage-backfill-snapshot");
    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);

    let first_invoke_id = "proxy-backfill-snapshot-first";
    let second_invoke_id = "proxy-backfill-snapshot-second";
    insert_proxy_backfill_row(&pool, first_invoke_id, &response_path).await;
    insert_proxy_backfill_row(&pool, second_invoke_id, &response_path).await;

    let first_id: i64 = sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
        .bind(first_invoke_id)
        .fetch_one(&pool)
        .await
        .expect("query first id");
    let second_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1")
            .bind(second_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query second id");

    let summary_first = backfill_proxy_usage_tokens_up_to_id(&pool, first_id, None)
        .await
        .expect("backfill up to first id should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);

    let first_total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind(first_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query first row tokens");
    let second_total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind(second_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query second row tokens");
    assert_eq!(first_total_tokens, Some(110));
    assert_eq!(second_total_tokens, None);

    let summary_second = backfill_proxy_usage_tokens_up_to_id(&pool, second_id, None)
        .await
        .expect("backfill up to second id should succeed");
    assert_eq!(summary_second.scanned, 1);
    assert_eq!(summary_second.updated, 1);

    let second_total_tokens_after: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind(second_invoke_id)
            .fetch_one(&pool)
            .await
            .expect("query second row tokens after second backfill");
    assert_eq!(second_total_tokens_after, Some(110));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_updates_dated_model_alias_and_is_idempotent() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-dated-model",
        Some("gpt-5.2-2025-12-11"),
        Some(1_000),
        Some(500),
    )
    .await;

    let catalog = PricingCatalog {
        version: "unit-cost-backfill".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary_first = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("first cost backfill should succeed");
    assert_eq!(summary_first.scanned, 1);
    assert_eq!(summary_first.updated, 1);
    assert_eq!(summary_first.skipped_unpriced_model, 0);

    let row = sqlx::query(
        "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-cost-backfill-dated-model")
    .fetch_one(&pool)
    .await
    .expect("query updated cost row");
    let expected = ((1_000.0 * 2.0) + (500.0 * 3.0)) / 1_000_000.0;
    assert!(
        (row.try_get::<Option<f64>, _>("cost")
            .expect("read cost")
            .expect("cost should exist")
            - expected)
            .abs()
            < 1e-12
    );
    assert_eq!(
        row.try_get::<Option<i64>, _>("cost_estimated")
            .expect("read cost_estimated"),
        Some(1)
    );
    assert_eq!(
        row.try_get::<Option<String>, _>("price_version")
            .expect("read price_version")
            .as_deref(),
        Some("unit-cost-backfill")
    );

    let summary_second = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("second cost backfill should be idempotent");
    assert_eq!(summary_second.scanned, 0);
    assert_eq!(summary_second.updated, 0);
}

#[tokio::test]
async fn backfill_proxy_missing_costs_skips_missing_model_or_usage_and_retries_unpriced_rows() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-missing-model",
        None,
        Some(1_000),
        Some(500),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-unpriced-model",
        Some("unknown-model"),
        Some(1_000),
        Some(500),
    )
    .await;
    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-backfill-missing-usage",
        Some("gpt-5.2"),
        None,
        None,
    )
    .await;

    let catalog = PricingCatalog {
        version: "unit-cost-backfill".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let summary = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("cost backfill should succeed");
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);
    assert_eq!(summary.skipped_unpriced_model, 1);
    let expected_attempt_version = pricing_backfill_attempt_version(&catalog);

    let unknown_row = sqlx::query(
        "SELECT cost, cost_estimated, price_version FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("proxy-cost-backfill-unpriced-model")
    .fetch_one(&pool)
    .await
    .expect("query unpriced model row");
    assert_eq!(
        unknown_row
            .try_get::<Option<f64>, _>("cost")
            .expect("read unknown cost"),
        None
    );
    assert_eq!(
        unknown_row
            .try_get::<Option<i64>, _>("cost_estimated")
            .expect("read unknown cost_estimated"),
        Some(0)
    );
    assert_eq!(
        unknown_row
            .try_get::<Option<String>, _>("price_version")
            .expect("read unknown price_version")
            .as_deref(),
        Some(expected_attempt_version.as_str())
    );

    let summary_same_version = backfill_proxy_missing_costs(&pool, &catalog)
        .await
        .expect("same-version cost backfill should skip attempted unpriced rows");
    assert_eq!(summary_same_version.scanned, 0);
    assert_eq!(summary_same_version.updated, 0);

    let updated_catalog_same_version = PricingCatalog {
        version: catalog.version.clone(),
        models: HashMap::from([
            (
                "gpt-5.2".to_string(),
                ModelPricing {
                    input_per_1m: 2.0,
                    output_per_1m: 3.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
            (
                "unknown-model".to_string(),
                ModelPricing {
                    input_per_1m: 4.0,
                    output_per_1m: 6.0,
                    cache_input_per_1m: None,
                    reasoning_per_1m: None,
                    source: "custom".to_string(),
                },
            ),
        ]),
    };
    let summary_same_version_after_pricing_update =
        backfill_proxy_missing_costs(&pool, &updated_catalog_same_version)
            .await
            .expect("same-version pricing update should retry previously unpriced rows");
    assert_eq!(summary_same_version_after_pricing_update.scanned, 1);
    assert_eq!(summary_same_version_after_pricing_update.updated, 1);
    assert_eq!(
        summary_same_version_after_pricing_update.skipped_unpriced_model,
        0
    );

    let unknown_cost_after_update: Option<f64> =
        sqlx::query_scalar("SELECT cost FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-cost-backfill-unpriced-model")
            .fetch_one(&pool)
            .await
            .expect("query unknown model cost after pricing update");
    let expected_unknown_cost = ((1_000.0 * 4.0) + (500.0 * 6.0)) / 1_000_000.0;
    assert!(
        (unknown_cost_after_update.expect("unknown cost should be backfilled")
            - expected_unknown_cost)
            .abs()
            < 1e-12
    );
}

#[test]
fn is_sqlite_lock_error_detects_structured_sqlite_codes() {
    let busy_code_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "5",
        },
    )));
    assert!(is_sqlite_lock_error(&busy_code_error));

    let sqlite_busy_name_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "SQLITE_BUSY",
        },
    )));
    assert!(is_sqlite_lock_error(&sqlite_busy_name_error));

    let non_lock_error = anyhow::Error::new(sqlx::Error::Database(Box::new(
        FakeSqliteCodeDatabaseError {
            message: "simulated sqlite driver failure",
            code: "SQLITE_CONSTRAINT",
        },
    )));
    assert!(!is_sqlite_lock_error(&non_lock_error));
}

#[tokio::test]
async fn build_sqlite_connect_options_enforces_wal_and_busy_timeout_defaults() {
    let temp_dir = make_temp_test_dir("sqlite-connect-options");
    let db_path = temp_dir.join("options.db");
    let db_url = sqlite_url_for_path(&db_path);

    let options = build_sqlite_connect_options(
        &db_url,
        Duration::from_secs(DEFAULT_SQLITE_BUSY_TIMEOUT_SECS),
    )
    .expect("build sqlite connect options");
    let mut conn = SqliteConnection::connect_with(&options)
        .await
        .expect("connect sqlite with options");

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode;")
        .fetch_one(&mut conn)
        .await
        .expect("read pragma journal_mode");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");

    let busy_timeout_ms: i64 = sqlx::query_scalar("PRAGMA busy_timeout;")
        .fetch_one(&mut conn)
        .await
        .expect("read pragma busy_timeout");
    assert_eq!(
        busy_timeout_ms,
        (DEFAULT_SQLITE_BUSY_TIMEOUT_SECS * 1_000) as i64
    );

    conn.close().await.expect("close sqlite connection");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_backfill_with_retry_succeeds_after_lock_release() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-success");
    let db_path = temp_dir.join("lock-success.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);
    insert_proxy_backfill_row(&pool, "proxy-lock-retry-success", &response_path).await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let backfill_task =
        tokio::spawn(async move { run_backfill_with_retry(&pool_for_task, None).await });

    tokio::time::sleep(Duration::from_millis(400)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");

    let summary = backfill_task
        .await
        .expect("join backfill task")
        .expect("backfill should succeed after retry");
    assert!(
        started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "expected retry delay to be applied"
    );
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-lock-retry-success")
            .fetch_one(&pool)
            .await
            .expect("query backfilled row");
    assert_eq!(total_tokens, Some(110));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_backfill_with_retry_fails_when_lock_persists() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-fail");
    let db_path = temp_dir.join("lock-fail.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let response_path = temp_dir.join("response.bin");
    write_backfill_response_payload(&response_path);
    insert_proxy_backfill_row(&pool, "proxy-lock-retry-fail", &response_path).await;

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let backfill_task =
        tokio::spawn(async move { run_backfill_with_retry(&pool_for_task, None).await });
    let err = backfill_task
        .await
        .expect("join backfill task")
        .expect_err("backfill should fail after lock retry exhaustion");
    assert!(
        started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "expected retry delay before final failure"
    );
    assert!(
        err.to_string().contains("failed after 2/2 attempt(s)"),
        "expected retry exhaustion context in error: {err:?}"
    );
    assert!(is_sqlite_lock_error(&err));

    let total_tokens: Option<i64> =
        sqlx::query_scalar("SELECT total_tokens FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-lock-retry-fail")
            .fetch_one(&pool)
            .await
            .expect("query locked row");
    assert_eq!(total_tokens, None);

    sqlx::query("ROLLBACK")
        .execute(&mut lock_conn)
        .await
        .expect("rollback lock holder");
    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_backfill_with_retry_does_not_retry_non_lock_errors() {
    let temp_dir = make_temp_test_dir("proxy-backfill-retry-non-lock");
    let db_path = temp_dir.join("non-lock.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");

    // Intentionally skip schema initialization to force a deterministic non-lock error.
    let started = Instant::now();
    let err = run_backfill_with_retry(&pool, None)
        .await
        .expect_err("backfill should fail immediately on non-lock errors");
    assert!(
        started.elapsed() < Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "non-lock errors should not wait for retry delay"
    );
    assert!(
        err.to_string().contains("failed after 1/2 attempt(s)"),
        "expected single-attempt context in error: {err:?}"
    );
    assert!(!is_sqlite_lock_error(&err));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table")
    }));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_cost_backfill_with_retry_succeeds_after_lock_release() {
    let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-success");
    let db_path = temp_dir.join("lock-success.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    insert_proxy_cost_backfill_row(
        &pool,
        "proxy-cost-lock-retry-success",
        Some("gpt-5.2-2025-12-11"),
        Some(2_000),
        Some(1_000),
    )
    .await;
    let catalog = PricingCatalog {
        version: "unit-cost-retry".to_string(),
        models: HashMap::from([(
            "gpt-5.2".to_string(),
            ModelPricing {
                input_per_1m: 2.0,
                output_per_1m: 3.0,
                cache_input_per_1m: None,
                reasoning_per_1m: None,
                source: "custom".to_string(),
            },
        )]),
    };

    let mut lock_conn = SqliteConnection::connect(&db_url)
        .await
        .expect("connect lock holder");
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut lock_conn)
        .await
        .expect("acquire sqlite write lock");

    let started = Instant::now();
    let pool_for_task = pool.clone();
    let catalog_for_task = catalog.clone();
    let backfill_task = tokio::spawn(async move {
        run_cost_backfill_with_retry(&pool_for_task, &catalog_for_task).await
    });

    tokio::time::sleep(Duration::from_millis(400)).await;
    sqlx::query("COMMIT")
        .execute(&mut lock_conn)
        .await
        .expect("release sqlite write lock");

    let summary = backfill_task
        .await
        .expect("join cost backfill task")
        .expect("cost backfill should succeed after retry");
    assert!(
        started.elapsed() >= Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "expected retry delay to be applied"
    );
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.updated, 1);

    let cost: Option<f64> =
        sqlx::query_scalar("SELECT cost FROM codex_invocations WHERE invoke_id = ?1")
            .bind("proxy-cost-lock-retry-success")
            .fetch_one(&pool)
            .await
            .expect("query backfilled cost row");
    assert!(cost.is_some());

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn run_cost_backfill_with_retry_does_not_retry_non_lock_errors() {
    let temp_dir = make_temp_test_dir("proxy-cost-backfill-retry-non-lock");
    let db_path = temp_dir.join("non-lock.db");
    let db_url = sqlite_url_for_path(&db_path);
    let connect_options = build_sqlite_connect_options(&db_url, Duration::from_millis(100))
        .expect("build sqlite options");
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(connect_options)
        .await
        .expect("connect sqlite pool");
    let catalog = PricingCatalog {
        version: "unit-cost-retry".to_string(),
        models: HashMap::new(),
    };

    // Intentionally skip schema initialization to force a deterministic non-lock error.
    let started = Instant::now();
    let err = run_cost_backfill_with_retry(&pool, &catalog)
        .await
        .expect_err("cost backfill should fail immediately on non-lock errors");
    assert!(
        started.elapsed() < Duration::from_secs(BACKFILL_LOCK_RETRY_DELAY_SECS),
        "non-lock errors should not wait for retry delay"
    );
    assert!(
        err.to_string().contains("failed after 1/2 attempt(s)"),
        "expected single-attempt context in error: {err:?}"
    );
    assert!(!is_sqlite_lock_error(&err));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .to_ascii_lowercase()
            .contains("no such table")
    }));

    pool.close().await;
    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn quota_latest_returns_degraded_when_empty() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect in-memory sqlite");
    ensure_schema(&pool)
        .await
        .expect("schema should initialize");

    let config = test_config();
    let http_clients = HttpClients::build(&config).expect("http clients");
    let semaphore = Arc::new(Semaphore::new(config.max_parallel_polls));
    let (broadcaster, _rx) = broadcast::channel(16);
    let state = Arc::new(AppState {
        config: config.clone(),
        pool,
        http_clients,
        broadcaster,
        broadcast_state_cache: Arc::new(Mutex::new(BroadcastStateCache::default())),
        proxy_summary_quota_broadcast_seq: Arc::new(AtomicU64::new(0)),
        proxy_summary_quota_broadcast_running: Arc::new(AtomicBool::new(false)),
        startup_ready: Arc::new(AtomicBool::new(true)),
        shutdown: CancellationToken::new(),
        semaphore,
        proxy_model_settings: Arc::new(RwLock::new(ProxyModelSettings::default())),
        proxy_model_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy: Arc::new(Mutex::new(ForwardProxyManager::new(
            ForwardProxySettings::default(),
            Vec::new(),
        ))),
        xray_supervisor: Arc::new(Mutex::new(XraySupervisor::new(
            config.xray_binary.clone(),
            config.xray_runtime_dir.clone(),
        ))),
        forward_proxy_settings_update_lock: Arc::new(Mutex::new(())),
        forward_proxy_subscription_refresh_lock: Arc::new(Mutex::new(())),
        pricing_settings_update_lock: Arc::new(Mutex::new(())),
        pricing_catalog: Arc::new(RwLock::new(PricingCatalog::default())),
        prompt_cache_conversation_cache: Arc::new(Mutex::new(
            PromptCacheConversationsCacheState::default(),
        )),
    });

    let Json(snapshot) = latest_quota_snapshot(State(state))
        .await
        .expect("route should succeed");

    assert!(!snapshot.is_active);
    assert_eq!(snapshot.total_requests, 0);
    assert_eq!(snapshot.total_cost, 0.0);
}

#[tokio::test]
async fn quota_latest_returns_seeded_historical_snapshot() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let captured_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    seed_quota_snapshot(&state.pool, &captured_at).await;

    let Json(snapshot) = latest_quota_snapshot(State(state))
        .await
        .expect("route should return seeded quota snapshot");

    assert_eq!(snapshot.captured_at, captured_at);
    let snapshot_json = serde_json::to_value(&snapshot).expect("serialize quota snapshot");
    assert!(
        snapshot_json["capturedAt"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z')),
        "serialized quota snapshot should emit UTC ISO timestamps"
    );
    assert!(snapshot.is_active);
    assert_eq!(snapshot.total_requests, 9);
    assert_f64_close(snapshot.total_cost, 10.0);
}

async fn insert_timeseries_invocation(
    pool: &SqlitePool,
    invoke_id: &str,
    occurred_at: &str,
    status: &str,
    t_upstream_ttfb_ms: Option<f64>,
) {
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            total_tokens,
            cost,
            t_upstream_ttfb_ms,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
    )
    .bind(invoke_id)
    .bind(occurred_at)
    .bind(SOURCE_PROXY)
    .bind(status)
    .bind(10_i64)
    .bind(0.01_f64)
    .bind(t_upstream_ttfb_ms)
    .bind("{}")
    .execute(pool)
    .await
    .expect("insert timeseries invocation");
}

fn assert_f64_close(actual: f64, expected: f64) {
    let diff = (actual - expected).abs();
    assert!(
        diff < 1e-6,
        "expected {expected}, got {actual}, diff={diff}"
    );
}

#[tokio::test]
async fn timeseries_includes_first_byte_avg_and_p95_for_success_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(5))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-1",
        &occurred_at,
        "success",
        Some(100.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-2",
        &occurred_at,
        "success",
        Some(200.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-3",
        &occurred_at,
        "success",
        Some(400.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-sample-failure",
        &occurred_at,
        "failed",
        Some(800.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 4)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 3);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        (100.0 + 200.0 + 400.0) / 3.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        380.0,
    );
}

#[tokio::test]
async fn timeseries_ignores_non_positive_or_missing_ttfb_samples() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let occurred_at = format_naive(
        (Utc::now() - ChronoDuration::minutes(10))
            .with_timezone(&Shanghai)
            .naive_local(),
    );
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-null",
        &occurred_at,
        "success",
        None,
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-zero",
        &occurred_at,
        "success",
        Some(0.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-negative",
        &occurred_at,
        "success",
        Some(-5.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-ignore-failed",
        &occurred_at,
        "failed",
        Some(250.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-keep-valid",
        &occurred_at,
        "success",
        Some(250.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1h".to_string(),
            bucket: Some("15m".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 5)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 1);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        250.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        250.0,
    );
}

#[tokio::test]
async fn timeseries_daily_bucket_includes_first_byte_stats() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    // Use "now" to avoid crossing local-day boundaries around midnight.
    let occurred_at = format_naive(Utc::now().with_timezone(&Shanghai).naive_local());
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-daily-1",
        &occurred_at,
        "success",
        Some(50.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-daily-2",
        &occurred_at,
        "success",
        Some(150.0),
    )
    .await;
    insert_timeseries_invocation(
        &state.pool,
        "ttfb-daily-failed",
        &occurred_at,
        "failed",
        Some(300.0),
    )
    .await;

    let Json(response) = fetch_timeseries(
        State(state),
        Query(TimeseriesQuery {
            range: "1d".to_string(),
            bucket: Some("1d".to_string()),
            settlement_hour: None,
            time_zone: Some("Asia/Shanghai".to_string()),
        }),
    )
    .await
    .expect("fetch timeseries");
    let bucket = response
        .points
        .iter()
        .find(|point| point.total_count >= 3)
        .expect("should include populated bucket");

    assert_eq!(bucket.first_byte_sample_count, 2);
    assert_f64_close(
        bucket.first_byte_avg_ms.expect("avg should be present"),
        100.0,
    );
    assert_f64_close(
        bucket.first_byte_p95_ms.expect("p95 should be present"),
        145.0,
    );
}

#[tokio::test]
async fn ensure_schema_adds_retention_columns_and_tables() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    sqlx::query(
        r#"
        CREATE TABLE codex_invocations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            invoke_id TEXT NOT NULL,
            occurred_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'xy',
            payload TEXT,
            raw_response TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(invoke_id, occurred_at)
        )
        "#,
    )
    .execute(&pool)
    .await
    .expect("create legacy invocation schema");

    ensure_schema(&pool).await.expect("ensure schema migration");

    let columns: HashSet<String> = sqlx::query("PRAGMA table_info('codex_invocations')")
        .fetch_all(&pool)
        .await
        .expect("inspect invocation columns")
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    assert!(columns.contains("detail_level"));
    assert!(columns.contains("detail_pruned_at"));
    assert!(columns.contains("detail_prune_reason"));

    let tables: HashSet<String> = sqlx::query_scalar(
        r#"
        SELECT name
        FROM sqlite_master
        WHERE type = 'table'
          AND name IN ('archive_batches', 'invocation_rollup_daily', 'startup_backfill_progress')
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load retention tables")
    .into_iter()
    .collect();
    assert!(tables.contains("archive_batches"));
    assert!(tables.contains("invocation_rollup_daily"));
    assert!(tables.contains("startup_backfill_progress"));
}

#[tokio::test]
async fn health_check_reports_starting_until_startup_is_ready() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18080").expect("valid upstream url"),
    )
    .await;

    state.startup_ready.store(false, Ordering::Release);
    let response = health_check(State(state.clone())).await.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "starting");

    state.startup_ready.store(true, Ordering::Release);
    let response = health_check(State(state)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read health body");
    assert_eq!(std::str::from_utf8(&body).expect("utf8 body"), "ok");
}

#[tokio::test]
async fn startup_backfill_progress_persists_terminal_missing_raw_cursor() {
    let state = test_state_with_openai_base(
        Url::parse("http://127.0.0.1:18081").expect("valid upstream url"),
    )
    .await;

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, payload, raw_response, request_raw_path
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("reasoning-missing-raw")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind("{}")
    .bind("{}")
    .bind("missing-reasoning-request.json")
    .execute(&state.pool)
    .await
    .expect("insert reasoning backfill row");

    let row_id: i64 =
        sqlx::query_scalar("SELECT id FROM codex_invocations WHERE invoke_id = ?1 LIMIT 1")
            .bind("reasoning-missing-raw")
            .fetch_one(&state.pool)
            .await
            .expect("fetch inserted row id");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("first startup backfill pass should succeed");

    let task_name =
        startup_backfill_task_progress_key(state.as_ref(), StartupBackfillTask::ReasoningEffort)
            .await;
    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after first pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 1);
    assert_eq!(progress.last_updated, 0);
    assert_eq!(progress.last_status, STARTUP_BACKFILL_STATUS_OK);

    sqlx::query("UPDATE startup_backfill_progress SET next_run_after = ?1 WHERE task_name = ?2")
        .bind(format_utc_iso(Utc::now() - ChronoDuration::seconds(1)))
        .bind(&task_name)
        .execute(&state.pool)
        .await
        .expect("force startup backfill task due again");

    run_startup_backfill_task_if_due(&state, StartupBackfillTask::ReasoningEffort)
        .await
        .expect("second startup backfill pass should skip previously scanned row");

    let progress = load_startup_backfill_progress(&state.pool, &task_name)
        .await
        .expect("load backfill progress after second pass");
    assert_eq!(progress.cursor_id, row_id);
    assert_eq!(progress.last_scanned, 0);
    assert_eq!(progress.last_updated, 0);
}

#[tokio::test]
async fn failure_classification_backfill_skips_success_rows_with_complete_defaults() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id,
            occurred_at,
            source,
            status,
            failure_class,
            is_actionable,
            raw_response
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind("success-no-kind")
    .bind("2026-03-09 00:00:00")
    .bind(SOURCE_PROXY)
    .bind("success")
    .bind(FAILURE_CLASS_NONE)
    .bind(0_i64)
    .bind("{}")
    .execute(&pool)
    .await
    .expect("insert success row");

    let outcome = backfill_failure_classification_from_cursor(&pool, 0, Some(10), None)
        .await
        .expect("run failure classification backfill");
    assert_eq!(outcome.summary.scanned, 0);
    assert_eq!(outcome.summary.updated, 0);
    assert_eq!(outcome.next_cursor_id, 0);
    assert!(!outcome.hit_budget);
}

#[tokio::test]
async fn failure_classification_backfill_from_cursor_respects_scan_limit() {
    let pool = SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("in-memory sqlite");
    ensure_schema(&pool).await.expect("ensure schema");

    for idx in 0..205 {
        sqlx::query(
            r#"
            INSERT INTO codex_invocations (
                invoke_id,
                occurred_at,
                source,
                status,
                error_message,
                raw_response
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        )
        .bind(format!("failure-classification-{idx}"))
        .bind("2026-03-09 00:00:00")
        .bind(SOURCE_PROXY)
        .bind("http_500")
        .bind("boom")
        .bind("{}")
        .execute(&pool)
        .await
        .expect("insert failure classification row");
    }

    let first = backfill_failure_classification_from_cursor(&pool, 0, Some(200), None)
        .await
        .expect("first bounded failure classification pass");
    assert_eq!(first.summary.scanned, 200);
    assert_eq!(first.summary.updated, 200);
    assert!(first.hit_budget);
    assert!(first.next_cursor_id > 0);

    let second =
        backfill_failure_classification_from_cursor(&pool, first.next_cursor_id, Some(200), None)
            .await
            .expect("second bounded failure classification pass");
    assert_eq!(second.summary.scanned, 5);
    assert_eq!(second.summary.updated, 5);
    assert!(!second.hit_budget);
}

#[tokio::test]
async fn retention_prunes_old_success_invocation_details_and_sweeps_orphans() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-prune").await;
    let response_raw = config.proxy_raw_dir.join("old-success-response.bin");
    fs::write(&response_raw, b"response-body").expect("write response raw");
    let request_missing = config.proxy_raw_dir.join("old-success-request.bin");
    let orphan = config.proxy_raw_dir.join("orphan.bin");
    fs::write(&orphan, b"orphan").expect("write orphan raw");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let occurred_at = shanghai_local_days_ago(31, 12, 0, 0);

    insert_retention_invocation(
        &pool,
        "old-success",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        Some(&request_missing),
        Some(&response_raw),
        Some(321),
        Some(1.23),
    )
    .await;

    let before_pruned_at = Utc::now() - ChronoDuration::seconds(5);
    let summary = run_data_retention_maintenance(&pool, &config, Some(false))
        .await
        .expect("run retention prune");
    let after_pruned_at = Utc::now() + ChronoDuration::seconds(5);
    assert_eq!(summary.invocation_details_pruned, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert_eq!(summary.orphan_raw_files_removed, 1);
    assert!(!response_raw.exists());
    assert!(!orphan.exists());

    let row = sqlx::query(
        r#"
        SELECT
            payload,
            raw_response,
            request_raw_path,
            response_raw_path,
            detail_level,
            detail_pruned_at,
            detail_prune_reason,
            total_tokens,
            cost,
            status
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-success")
    .fetch_one(&pool)
    .await
    .expect("load pruned invocation");
    assert_eq!(
        row.get::<String, _>("detail_level"),
        DETAIL_LEVEL_STRUCTURED_ONLY
    );
    assert!(row.get::<Option<String>, _>("detail_pruned_at").is_some());
    assert_eq!(
        row.get::<Option<String>, _>("detail_prune_reason")
            .as_deref(),
        Some(DETAIL_PRUNE_REASON_SUCCESS_OVER_30D)
    );
    assert!(row.get::<Option<String>, _>("payload").is_none());
    assert_eq!(row.get::<String, _>("raw_response"), "");
    assert!(row.get::<Option<String>, _>("request_raw_path").is_none());
    assert!(row.get::<Option<String>, _>("response_raw_path").is_none());
    assert_eq!(row.get::<Option<i64>, _>("total_tokens"), Some(321));
    assert_f64_close(row.get::<Option<f64>, _>("cost").unwrap_or_default(), 1.23);
    assert_eq!(
        row.get::<Option<String>, _>("status").as_deref(),
        Some("success")
    );

    let detail_pruned_at = row
        .get::<Option<String>, _>("detail_pruned_at")
        .expect("detail_pruned_at should be populated");
    let detail_pruned_at = local_naive_to_utc(
        parse_shanghai_local_naive(&detail_pruned_at)
            .expect("detail_pruned_at should be shanghai-local"),
        Shanghai,
    )
    .with_timezone(&Utc);
    assert!(detail_pruned_at >= before_pruned_at);
    assert!(detail_pruned_at <= after_pruned_at);

    let batch = sqlx::query(
        r#"
        SELECT file_path, row_count, status
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load prune archive batch");
    let file_path = PathBuf::from(batch.get::<String, _>("file_path"));
    assert!(file_path.exists());
    assert_eq!(batch.get::<String, _>("status"), ARCHIVE_STATUS_COMPLETED);
    assert_eq!(batch.get::<i64, _>("row_count"), 1);

    let archive_db_path = temp_dir.join("retention-prune-archive.sqlite");
    inflate_gzip_sqlite_file(&file_path, &archive_db_path).expect("inflate prune archive");
    let archive_pool = SqlitePool::connect(&sqlite_url_for_path(&archive_db_path))
        .await
        .expect("open prune archive sqlite");
    let archived = sqlx::query(
        r#"
        SELECT payload, raw_response, detail_level, detail_pruned_at, detail_prune_reason
        FROM codex_invocations
        WHERE invoke_id = ?1
        "#,
    )
    .bind("old-success")
    .fetch_one(&archive_pool)
    .await
    .expect("load archived pre-prune invocation");
    assert_eq!(
        archived.get::<Option<String>, _>("payload").as_deref(),
        Some("{\"endpoint\":\"/v1/responses\"}")
    );
    assert_eq!(archived.get::<String, _>("raw_response"), "{\"ok\":true}");
    assert_eq!(archived.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
    assert!(
        archived
            .get::<Option<String>, _>("detail_pruned_at")
            .is_none()
    );
    assert!(
        archived
            .get::<Option<String>, _>("detail_prune_reason")
            .is_none()
    );
    archive_pool.close().await;

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_old_invocations_without_changing_summary_all() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-archive").await;
    let old_response = config.proxy_raw_dir.join("old-archive-response.bin");
    fs::write(&old_response, b"archive-response").expect("write archive raw");
    let old_occurred_at = shanghai_local_days_ago(91, 10, 0, 0);
    let old_failed_at = shanghai_local_days_ago(92, 11, 0, 0);
    let recent_at = shanghai_local_days_ago(5, 15, 0, 0);

    insert_retention_invocation(
        &pool,
        "archive-old-success",
        &old_occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&old_response),
        Some(100),
        Some(0.5),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "archive-old-failed",
        &old_failed_at,
        SOURCE_PROXY,
        "failed",
        Some("{\"endpoint\":\"/v1/chat/completions\"}"),
        "{\"error\":true}",
        None,
        None,
        Some(50),
        Some(0.25),
    )
    .await;
    insert_retention_invocation(
        &pool,
        "archive-recent",
        &recent_at,
        SOURCE_PROXY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        None,
        Some(70),
        Some(0.75),
    )
    .await;

    let before = query_combined_totals(&pool, None, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query totals before retention");
    let summary = run_data_retention_maintenance(&pool, &config, Some(false))
        .await
        .expect("run retention archive");
    let after = query_combined_totals(&pool, None, StatsFilter::All, InvocationSourceScope::All)
        .await
        .expect("query totals after retention");

    assert_eq!(summary.invocation_rows_archived, 2);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(before.total_count, after.total_count);
    assert_eq!(before.success_count, after.success_count);
    assert_eq!(before.failure_count, after.failure_count);
    assert_eq!(before.total_tokens, after.total_tokens);
    assert_f64_close(before.total_cost, after.total_cost);
    assert!(!old_response.exists());

    let live_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM codex_invocations")
        .fetch_one(&pool)
        .await
        .expect("count live invocations");
    assert_eq!(live_count, 1);

    let rollup = sqlx::query(
        r#"
        SELECT total_count, success_count, failure_count, total_tokens, total_cost
        FROM invocation_rollup_daily
        WHERE stats_date = ?1 AND source = ?2
        "#,
    )
    .bind(&old_occurred_at[..10])
    .bind(SOURCE_XY)
    .fetch_one(&pool)
    .await
    .expect("load invocation rollup row");
    assert_eq!(rollup.get::<i64, _>("total_count"), 1);
    assert_eq!(rollup.get::<i64, _>("success_count"), 1);
    assert_eq!(rollup.get::<i64, _>("failure_count"), 0);
    assert_eq!(rollup.get::<i64, _>("total_tokens"), 100);
    assert_f64_close(rollup.get::<f64, _>("total_cost"), 0.5);

    let batch = sqlx::query(
        r#"
        SELECT file_path, row_count, status
        FROM archive_batches
        WHERE dataset = 'codex_invocations'
        "#,
    )
    .fetch_one(&pool)
    .await
    .expect("load invocation archive batch");
    let file_path = PathBuf::from(batch.get::<String, _>("file_path"));
    assert!(file_path.exists());
    assert!(batch.get::<i64, _>("row_count") >= 2);
    assert_eq!(batch.get::<String, _>("status"), ARCHIVE_STATUS_COMPLETED);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_archives_forward_proxy_attempts_and_stats_snapshots() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-timestamped").await;
    let old_attempt = Utc::now() - ChronoDuration::days(35);
    let recent_attempt = Utc::now() - ChronoDuration::days(1);
    seed_forward_proxy_attempt_at(&pool, "proxy-old", old_attempt, true).await;
    seed_forward_proxy_attempt_at(&pool, "proxy-new", recent_attempt, true).await;

    let old_captured_at = utc_naive_from_shanghai_local_days_ago(35, 8, 0, 0);
    let recent_captured_at = utc_naive_from_shanghai_local_days_ago(1, 8, 0, 0);
    insert_stats_source_snapshot_row(&pool, &old_captured_at, &old_captured_at[..10]).await;
    insert_stats_source_snapshot_row(&pool, &recent_captured_at, &recent_captured_at[..10]).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false))
        .await
        .expect("run timestamped retention");
    assert_eq!(summary.forward_proxy_attempt_rows_archived, 1);
    assert_eq!(summary.stats_source_snapshot_rows_archived, 1);

    let remaining_old_attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM forward_proxy_attempts WHERE occurred_at < ?1")
            .bind(shanghai_utc_cutoff_string(
                config.forward_proxy_attempts_retention_days,
            ))
            .fetch_one(&pool)
            .await
            .expect("count old forward proxy attempts");
    assert_eq!(remaining_old_attempts, 0);

    let remaining_old_snapshots: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM stats_source_snapshots WHERE captured_at < ?1")
            .bind(shanghai_utc_cutoff_string(
                config.stats_source_snapshots_retention_days,
            ))
            .fetch_one(&pool)
            .await
            .expect("count old stats snapshots");
    assert_eq!(remaining_old_snapshots, 0);

    let datasets: HashSet<String> = sqlx::query_scalar(
        r#"
        SELECT dataset
        FROM archive_batches
        WHERE dataset IN ('forward_proxy_attempts', 'stats_source_snapshots')
        "#,
    )
    .fetch_all(&pool)
    .await
    .expect("load timestamped archive batch datasets")
    .into_iter()
    .collect();
    assert!(datasets.contains("forward_proxy_attempts"));
    assert!(datasets.contains("stats_source_snapshots"));

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_compacts_old_quota_snapshots_by_shanghai_day() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-quota").await;
    let same_day_early = utc_naive_from_shanghai_local_days_ago(40, 8, 0, 0);
    let same_day_late = utc_naive_from_shanghai_local_days_ago(40, 23, 0, 0);
    let next_day = utc_naive_from_shanghai_local_days_ago(39, 9, 0, 0);
    seed_quota_snapshot(&pool, &same_day_early).await;
    seed_quota_snapshot(&pool, &same_day_late).await;
    seed_quota_snapshot(&pool, &next_day).await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(false))
        .await
        .expect("run quota compaction");
    assert_eq!(summary.quota_snapshot_rows_archived, 1);

    let remaining: Vec<String> = sqlx::query_scalar(
        "SELECT captured_at FROM codex_quota_snapshots ORDER BY captured_at ASC",
    )
    .fetch_all(&pool)
    .await
    .expect("load remaining quota snapshots");
    assert_eq!(remaining, vec![same_day_late.clone(), next_day.clone()]);

    let quota_batch_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM archive_batches WHERE dataset = 'codex_quota_snapshots'",
    )
    .fetch_one(&pool)
    .await
    .expect("count quota archive batches");
    assert_eq!(quota_batch_count, 1);

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_orphan_sweep_skips_fresh_raw_files() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-orphan-grace").await;
    let orphan = config.proxy_raw_dir.join("fresh-orphan.bin");
    fs::write(&orphan, b"fresh-orphan").expect("write fresh orphan");

    let summary = run_data_retention_maintenance(&pool, &config, Some(false))
        .await
        .expect("run retention with fresh orphan");
    assert_eq!(summary.orphan_raw_files_removed, 0);
    assert!(orphan.exists());

    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test(flavor = "current_thread")]
async fn retention_orphan_sweep_anchors_relative_raw_dir_to_database_parent() {
    let _guard = APP_CONFIG_ENV_LOCK.lock().expect("cwd lock");
    let temp_dir = make_temp_test_dir("retention-orphan-db-parent");
    let db_root = temp_dir.join("db-root");
    let cwd_root = temp_dir.join("cwd-root");
    fs::create_dir_all(&db_root).expect("create db root");
    fs::create_dir_all(&cwd_root).expect("create cwd root");
    let _cwd_guard = CurrentDirGuard::change_to(&cwd_root);

    let db_path = db_root.join("codex-vibe-monitor.db");
    fs::File::create(&db_path).expect("create sqlite file");
    let pool = SqlitePool::connect(&sqlite_url_for_path(&db_path))
        .await
        .expect("connect retention sqlite");
    ensure_schema(&pool).await.expect("ensure retention schema");

    let mut config = test_config();
    config.database_path = db_path;
    config.proxy_raw_dir = PathBuf::from("proxy_raw_payloads");

    let anchored_dir = config.resolved_proxy_raw_dir();
    fs::create_dir_all(&anchored_dir).expect("create anchored raw dir");
    let anchored_orphan = anchored_dir.join("anchored-orphan.bin");
    fs::write(&anchored_orphan, b"anchored-orphan").expect("write anchored orphan");
    set_file_mtime_seconds_ago(&anchored_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

    let cwd_raw_dir = cwd_root.join("proxy_raw_payloads");
    fs::create_dir_all(&cwd_raw_dir).expect("create cwd raw dir");
    let cwd_orphan = cwd_raw_dir.join("cwd-orphan.bin");
    fs::write(&cwd_orphan, b"cwd-orphan").expect("write cwd orphan");
    set_file_mtime_seconds_ago(&cwd_orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);

    let removed = sweep_orphan_proxy_raw_files(&pool, &config, None, false)
        .await
        .expect("run orphan sweep");

    assert_eq!(removed, 1);
    assert!(
        !anchored_orphan.exists(),
        "orphan sweep should clean the database-anchored raw dir"
    );
    assert!(
        cwd_orphan.exists(),
        "orphan sweep should stop scanning cwd-relative stray files"
    );

    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[tokio::test]
async fn retention_dry_run_does_not_mutate_database_or_files() {
    let (pool, config, temp_dir) = retention_test_pool_and_config("retention-dry-run").await;
    let response_raw = config.proxy_raw_dir.join("dry-run-response.bin");
    let orphan = config.proxy_raw_dir.join("dry-run-orphan.bin");
    fs::write(&response_raw, b"dry-run-response").expect("write dry-run response raw");
    fs::write(&orphan, b"dry-run-orphan").expect("write dry-run orphan");
    set_file_mtime_seconds_ago(&orphan, DEFAULT_ORPHAN_SWEEP_MIN_AGE_SECS + 60);
    let occurred_at = shanghai_local_days_ago(91, 7, 0, 0);
    insert_retention_invocation(
        &pool,
        "dry-run-old",
        &occurred_at,
        SOURCE_XY,
        "success",
        Some("{\"endpoint\":\"/v1/responses\"}"),
        "{\"ok\":true}",
        None,
        Some(&response_raw),
        Some(111),
        Some(0.9),
    )
    .await;

    let summary = run_data_retention_maintenance(&pool, &config, Some(true))
        .await
        .expect("run dry-run retention");
    assert!(summary.dry_run);
    assert_eq!(summary.invocation_rows_archived, 1);
    assert_eq!(summary.archive_batches_touched, 1);
    assert_eq!(summary.raw_files_removed, 1);
    assert_eq!(summary.orphan_raw_files_removed, 1);
    assert!(response_raw.exists());
    assert!(orphan.exists());

    let row = sqlx::query(
        "SELECT detail_level, payload, raw_response FROM codex_invocations WHERE invoke_id = ?1",
    )
    .bind("dry-run-old")
    .fetch_one(&pool)
    .await
    .expect("load dry-run invocation");
    assert_eq!(row.get::<String, _>("detail_level"), DETAIL_LEVEL_FULL);
    assert!(row.get::<Option<String>, _>("payload").is_some());
    assert_eq!(row.get::<String, _>("raw_response"), "{\"ok\":true}");

    let archive_batch_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM archive_batches")
        .fetch_one(&pool)
        .await
        .expect("count dry-run archive batches");
    assert_eq!(archive_batch_count, 0);

    let archive_files = fs::read_dir(&config.archive_dir)
        .expect("read archive dir")
        .count();
    assert_eq!(archive_files, 0);

    cleanup_temp_test_dir(&temp_dir);
}

async fn spawn_test_crs_stats_server(
    release_request: Arc<Notify>,
    request_count: Arc<AtomicUsize>,
) -> (String, JoinHandle<()>) {
    let app = Router::new().route(
        "/apiStats/api/user-model-stats",
        post(move || {
            let release_request = release_request.clone();
            let request_count = request_count.clone();
            async move {
                request_count.fetch_add(1, Ordering::SeqCst);
                release_request.notified().await;
                (
                    StatusCode::OK,
                    Json(json!({
                        "success": true,
                        "period": "daily",
                        "data": [],
                    })),
                )
            }
        }),
    );

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind crs stats test server");
    let addr = listener.local_addr().expect("crs stats test server addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("crs stats test server should run");
    });

    (format!("http://{addr}/"), handle)
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_child_process_prefers_sigterm_when_process_exits_cleanly() {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap 'exit 0' TERM; while :; do sleep 0.1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sigterm-friendly child");

    let outcome = terminate_child_process(&mut child, Duration::from_secs(1), "test-child").await;

    assert_eq!(outcome, ChildTerminationOutcome::Graceful);
    assert!(
        child
            .try_wait()
            .expect("poll child after terminate")
            .is_some()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn terminate_child_process_falls_back_to_force_kill_when_grace_period_is_exhausted() {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg("trap 'exit 0' TERM; while :; do sleep 0.1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child for forced shutdown fallback");

    let outcome = terminate_child_process(&mut child, Duration::ZERO, "test-child").await;

    assert_eq!(outcome, ChildTerminationOutcome::Forced);
    assert!(
        child
            .try_wait()
            .expect("poll child after force kill")
            .is_some()
    );
}

#[tokio::test]
async fn http_server_graceful_shutdown_stops_accepting_new_connections() {
    let state = test_state_from_config(test_config(), false).await;
    let (addr, server_handle) = spawn_http_server(state.clone())
        .await
        .expect("spawn http server");

    let healthy_response = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect("health endpoint should respond before shutdown");
    assert_eq!(healthy_response.status(), StatusCode::OK);

    state.shutdown.cancel();
    server_handle.await.expect("http server task should join");

    let err = reqwest::get(format!("http://{addr}/health"))
        .await
        .expect_err("server should stop accepting new connections after shutdown");
    assert!(err.is_connect() || err.is_timeout());
}

#[tokio::test]
async fn run_runtime_until_shutdown_waits_for_inflight_scheduler_poll() {
    let release_request = Arc::new(Notify::new());
    let request_count = Arc::new(AtomicUsize::new(0));
    let (crs_base, crs_handle) =
        spawn_test_crs_stats_server(release_request.clone(), request_count.clone()).await;

    let mut config = test_config();
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse(&crs_base).expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    config.request_timeout = Duration::from_secs(5);
    config.poll_interval = Duration::from_millis(25);
    config.max_parallel_polls = 1;
    let state = test_state_from_config(config, false).await;

    let shutdown = Arc::new(Notify::new());
    let shutdown_for_runtime = shutdown.clone();
    let state_for_runtime = state.clone();
    let runtime_handle = tokio::spawn(async move {
        run_runtime_until_shutdown(state_for_runtime, Instant::now(), async move {
            shutdown_for_runtime.notified().await;
        })
        .await
    });

    tokio::time::timeout(Duration::from_secs(2), async {
        while request_count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("scheduler should start an in-flight poll");
    shutdown.notify_waiters();
    tokio::time::sleep(Duration::from_millis(100)).await;

    assert!(
        !runtime_handle.is_finished(),
        "runtime should wait for the in-flight scheduler poll to finish"
    );
    assert_eq!(request_count.load(Ordering::SeqCst), 1);

    release_request.notify_waiters();
    runtime_handle
        .await
        .expect("runtime task should join")
        .expect("runtime should shutdown cleanly");

    assert!(state.shutdown.is_cancelled());
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    crs_handle.abort();
}

#[tokio::test]
async fn run_runtime_until_shutdown_skips_startup_work_when_shutdown_is_already_requested() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let release_request = Arc::new(Notify::new());
    let (crs_base, crs_handle) =
        spawn_test_crs_stats_server(release_request.clone(), request_count.clone()).await;

    let mut config = test_config();
    config.crs_stats = Some(CrsStatsConfig {
        base_url: Url::parse(&crs_base).expect("valid crs base url"),
        api_id: "test-api".to_string(),
        period: "daily".to_string(),
        poll_interval: Duration::from_secs(3600),
    });
    config.request_timeout = Duration::from_secs(5);
    config.poll_interval = Duration::from_millis(25);
    config.max_parallel_polls = 1;
    let state = test_state_from_config(config, false).await;

    run_runtime_until_shutdown(state.clone(), Instant::now(), async {})
        .await
        .expect("runtime should exit cleanly when shutdown is already requested");

    assert!(state.shutdown.is_cancelled());
    assert_eq!(request_count.load(Ordering::SeqCst), 0);
    release_request.notify_waiters();
    crs_handle.abort();
}

#[tokio::test]
async fn run_runtime_until_shutdown_skips_xray_route_sync_when_shutdown_is_already_requested() {
    let runtime_dir = make_temp_test_dir("runtime-shutdown-xray-sync");
    fs::remove_dir_all(&runtime_dir).expect("remove temp runtime dir before startup");

    let mut config = test_config();
    config.xray_binary = "/path/to/non-existent-xray".to_string();
    config.xray_runtime_dir = runtime_dir.clone();
    let state = test_state_from_config(config, false).await;

    {
        let mut manager = state.forward_proxy.lock().await;
        manager.apply_settings(ForwardProxySettings {
            proxy_urls: vec!["vless://11111111-1111-1111-1111-111111111111@vless.example.com:443?security=tls&type=ws&path=%2Fws&host=cdn.vless.example.com#vless".to_string()],
            subscription_urls: Vec::new(),
            subscription_update_interval_secs: 3600,
            insert_direct: true,
        });
    }

    run_runtime_until_shutdown(state.clone(), Instant::now(), async {})
        .await
        .expect("runtime should exit cleanly when shutdown is already requested");

    assert!(state.shutdown.is_cancelled());
    assert!(
        !runtime_dir.exists(),
        "shutdown should skip xray route sync side effects when startup never begins"
    );
}

#[tokio::test]
async fn bootstrap_probe_round_skips_work_when_shutdown_is_in_progress() {
    let (proxy_url, proxy_handle) = spawn_test_forward_proxy_status(StatusCode::OK).await;
    let normalized_proxy =
        normalize_single_proxy_url(&proxy_url).expect("normalize forward proxy url");
    let state = test_state_with_openai_base(
        Url::parse("http://probe-target.example/").expect("valid upstream base url"),
    )
    .await;
    state.shutdown.cancel();

    spawn_forward_proxy_bootstrap_probe_round(
        state.clone(),
        vec![ForwardProxyEndpoint {
            key: normalized_proxy.clone(),
            source: FORWARD_PROXY_SOURCE_MANUAL.to_string(),
            display_name: normalized_proxy.clone(),
            protocol: ForwardProxyProtocol::Http,
            endpoint_url: Some(Url::parse(&normalized_proxy).expect("valid normalized proxy url")),
            raw_url: Some(normalized_proxy.clone()),
        }],
        "test-shutdown",
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    let probe_count =
        count_forward_proxy_probe_attempts(&state.pool, &normalized_proxy, None).await;
    assert_eq!(probe_count, 0);

    proxy_handle.abort();
}

#[tokio::test]
async fn persist_and_broadcast_proxy_capture_skips_summary_worker_during_shutdown() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let mut rx = state.broadcaster.subscribe();
    state.shutdown.cancel();

    persist_and_broadcast_proxy_capture(
        state.as_ref(),
        Instant::now(),
        test_proxy_capture_record("shutdown-broadcast", &format_utc_iso(Utc::now())),
    )
    .await
    .expect("persist proxy capture during shutdown");

    let payload = tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("shutdown path should still emit the persisted record")
        .expect("broadcast channel should stay open");
    assert!(
        matches!(payload, BroadcastPayload::Records { .. }),
        "shutdown path should keep the live record event aligned with persisted data"
    );
    assert!(
        !state
            .proxy_summary_quota_broadcast_running
            .load(Ordering::Acquire),
        "summary/quota broadcast worker should not stay active during shutdown"
    );
}
