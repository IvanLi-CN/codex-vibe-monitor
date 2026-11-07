use super::*;
use hmac::{Hmac, Mac};

const ADMIN_AUTH_REALM: &str = "Basic realm=\"CVM Admin\", charset=\"UTF-8\"";
const MAX_AUTHORIZATION_HEADER_BYTES: usize = 8192;
const MAX_DECODED_CREDENTIAL_BYTES: usize = 4096;
const MAX_CREDENTIAL_COMPONENT_BYTES: usize = 4096;
const PUBLIC_BLOG_RUNTIME_PATH: &str = "/api/public/metrics/v1/codex-vibe-monitor";

pub(crate) fn apply_private_admin_auth(
    router: Router,
    admin_auth: Option<AdminAuthConfig>,
) -> Router {
    router.layer(axum::middleware::from_fn_with_state(
        admin_auth,
        private_admin_auth_middleware,
    ))
}

pub(crate) fn is_private_admin_path(path: &str) -> bool {
    if path == "/events" || path == "/health/analytics" {
        return true;
    }
    if path == "/health"
        || path == "/api/version"
        || path == "/v1"
        || path.starts_with("/v1/")
        || path == PUBLIC_BLOG_RUNTIME_PATH
        || is_external_api_route(path)
    {
        return false;
    }
    path == "/api" || path.starts_with("/api/")
}

fn is_external_api_route(path: &str) -> bool {
    let segments = path.split('/').skip(1).collect::<Vec<_>>();
    if segments.len() == 6 {
        return segments[..5] == ["api", "external", "v1", "upstream-accounts", "oauth"]
            && !segments[5].is_empty();
    }
    segments.len() == 7
        && segments[..5] == ["api", "external", "v1", "upstream-accounts", "oauth"]
        && !segments[5].is_empty()
        && segments[6] == "relogin"
}

async fn private_admin_auth_middleware(
    State(admin_auth): State<Option<AdminAuthConfig>>,
    mut request: Request<Body>,
    next: axum::middleware::Next,
) -> Response {
    let private = is_private_admin_path(request.uri().path());
    if private {
        match admin_auth.as_ref() {
            Some(AdminAuthConfig::Configured { credential_tag })
                if has_valid_basic_credentials(request.headers(), credential_tag) => {}
            Some(AdminAuthConfig::Configured { .. }) => return unauthorized_response(),
            Some(AdminAuthConfig::Unavailable) | None => return auth_unavailable_response(),
        }
    }

    // A valid admin credential authorizes this service only. Remove Basic credentials before
    // any downstream handler, including the public proxy and the independently-authenticated
    // external API. Other authorization schemes remain available to their own contract.
    strip_basic_authorization(request.headers_mut());

    let mut response = next.run(request).await;
    if private {
        apply_no_store(&mut response);
    }
    response
}

fn has_valid_basic_credentials(headers: &HeaderMap, expected: &[u8; 32]) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let Some(value) = values.next() else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    let Some((username, password)) = decode_basic_credentials(value) else {
        return false;
    };
    let mut verifier = Hmac::<Sha256>::new_from_slice(ADMIN_AUTH_HMAC_KEY)
        .expect("static admin auth HMAC key must be valid");
    verifier.update(username.as_bytes());
    verifier.update(&[0]);
    verifier.update(password.as_bytes());
    verifier.verify_slice(expected).is_ok()
}

fn decode_basic_credentials(value: &HeaderValue) -> Option<(String, String)> {
    let raw = value.to_str().ok()?;
    if raw.len() > MAX_AUTHORIZATION_HEADER_BYTES {
        return None;
    }
    let (scheme, encoded) = raw.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Basic") || encoded.is_empty() {
        return None;
    }
    if encoded.len() > MAX_AUTHORIZATION_HEADER_BYTES
        || encoded.len() % 4 != 0
        || !strict_base64_text(encoded)
    {
        return None;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if decoded.is_empty() || decoded.len() > MAX_DECODED_CREDENTIAL_BYTES {
        return None;
    }
    let decoded = std::str::from_utf8(&decoded).ok()?;
    let (username, password) = decoded.split_once(':')?;
    if !valid_credential_component(username, MAX_CREDENTIAL_COMPONENT_BYTES)
        || !valid_credential_component(password, MAX_CREDENTIAL_COMPONENT_BYTES)
    {
        return None;
    }
    Some((username.to_string(), password.to_string()))
}

fn strict_base64_text(encoded: &str) -> bool {
    let bytes = encoded.as_bytes();
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 {
        return false;
    }
    let content_len = bytes.len() - padding;
    if bytes[..content_len].contains(&b'=') {
        return false;
    }
    bytes[..content_len]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'+' | b'/'))
}

fn valid_credential_component(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_bytes
        && value.bytes().all(|byte| !byte.is_ascii_control())
}

fn is_basic_authorization(value: &HeaderValue) -> bool {
    value
        .to_str()
        .ok()
        .and_then(|raw| raw.split_ascii_whitespace().next())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("Basic"))
}

fn strip_basic_authorization(headers: &mut HeaderMap) {
    let values = headers
        .get_all(header::AUTHORIZATION)
        .iter()
        .filter(|value| !is_basic_authorization(value))
        .cloned()
        .collect::<Vec<_>>();
    let original_count = headers.get_all(header::AUTHORIZATION).iter().count();
    if values.len() == original_count {
        return;
    }
    headers.remove(header::AUTHORIZATION);
    for value in values {
        headers.append(header::AUTHORIZATION, value);
    }
}

fn apply_no_store(response: &mut Response) {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
}

fn unauthorized_response() -> Response {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static(ADMIN_AUTH_REALM),
    );
    apply_no_store(&mut response);
    response
}

fn auth_unavailable_response() -> Response {
    let mut response = StatusCode::SERVICE_UNAVAILABLE.into_response();
    apply_no_store(&mut response);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn observed_headers(headers: HeaderMap) -> Response {
        if headers.contains_key(header::AUTHORIZATION) {
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        } else {
            StatusCode::OK.into_response()
        }
    }

    async fn observed_external_auth(headers: HeaderMap) -> Response {
        if headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some("Bearer external-secret")
        {
            StatusCode::OK.into_response()
        } else {
            StatusCode::UNAUTHORIZED.into_response()
        }
    }

    fn test_router(admin_auth: Option<AdminAuthConfig>) -> Router {
        let router = Router::new()
            .route("/api/stats", get(observed_headers))
            .route("/events", get(observed_headers))
            .route("/health", get(observed_headers))
            .route("/api/version", get(observed_headers))
            .route("/health/analytics", get(observed_headers))
            .route(
                "/api/public/metrics/v1/codex-vibe-monitor",
                get(observed_headers),
            )
            .route(
                "/api/external/v1/upstream-accounts/oauth/account-1",
                get(observed_external_auth),
            )
            .route("/v1/models", get(observed_headers))
            .with_state(());
        apply_private_admin_auth(router, admin_auth)
    }

    fn basic_header(username: &str, password: &str) -> HeaderValue {
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        HeaderValue::from_str(&format!("Basic {encoded}")).expect("valid basic header")
    }

    #[tokio::test]
    async fn missing_configuration_closes_private_routes_without_cache() {
        for path in ["/api/stats", "/health/analytics", "/events?resume=cursor-1"] {
            let response = test_router(None)
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("valid request"),
                )
                .await
                .expect("router response");

            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store",
                "{path}"
            );
            assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
        }
    }

    #[tokio::test]
    async fn configured_private_routes_require_one_valid_basic_header() {
        let admin_auth = AdminAuthConfig::from_credentials("admin".to_string(), "secret").unwrap();
        let router = test_router(Some(admin_auth));

        let missing = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/events")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(missing.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            missing.headers()[header::WWW_AUTHENTICATE],
            ADMIN_AUTH_REALM
        );

        let malformed = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health/analytics")
                    .header(header::AUTHORIZATION, "Basic !!!")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(malformed.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(malformed.headers()[header::CACHE_CONTROL], "no-store");

        let mut duplicate = Request::builder()
            .uri("/api/stats")
            .body(Body::empty())
            .expect("valid request");
        duplicate
            .headers_mut()
            .append(header::AUTHORIZATION, basic_header("admin", "secret"));
        duplicate
            .headers_mut()
            .append(header::AUTHORIZATION, basic_header("admin", "secret"));
        let duplicate = router
            .clone()
            .oneshot(duplicate)
            .await
            .expect("router response");
        assert_eq!(duplicate.status(), StatusCode::UNAUTHORIZED);

        let authorized = router
            .oneshot(
                Request::builder()
                    .uri("/api/stats")
                    .header(header::AUTHORIZATION, basic_header("admin", "secret"))
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(authorized.status(), StatusCode::OK);
        assert_eq!(authorized.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn public_and_proxy_routes_do_not_use_admin_basic_gate() {
        let router = test_router(None);
        for path in [
            "/health",
            "/api/version",
            "/api/public/metrics/v1/codex-vibe-monitor",
            "/v1/models",
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .header(header::AUTHORIZATION, basic_header("admin", "secret"))
                        .body(Body::empty())
                        .expect("valid request"),
                )
                .await
                .expect("router response");
            assert_eq!(response.status(), StatusCode::OK, "{path}");
        }

        let external_basic = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/external/v1/upstream-accounts/oauth/account-1")
                    .header(header::AUTHORIZATION, basic_header("admin", "secret"))
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(external_basic.status(), StatusCode::UNAUTHORIZED);

        let external_bearer = router
            .oneshot(
                Request::builder()
                    .uri("/api/external/v1/upstream-accounts/oauth/account-1")
                    .header(header::AUTHORIZATION, "Bearer external-secret")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("router response");
        assert_eq!(external_bearer.status(), StatusCode::OK);
    }

    #[test]
    fn private_path_allowlist_does_not_open_unowned_api_prefixes() {
        assert!(!is_private_admin_path("/health"));
        assert!(is_private_admin_path("/health/analytics"));
        assert!(!is_private_admin_path(PUBLIC_BLOG_RUNTIME_PATH));
        assert!(is_private_admin_path("/api/public/other"));
        assert!(!is_private_admin_path(
            "/api/external/v1/upstream-accounts/oauth/account-1"
        ));
        assert!(is_private_admin_path("/api/external/unknown"));
        assert!(!is_private_admin_path("/v1/models"));
        assert!(is_private_admin_path("/api/stats"));
    }
}
