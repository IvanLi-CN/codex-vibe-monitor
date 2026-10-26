use super::*;

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
