use super::*;

#[test]
fn vless_xhttp_identity_normalizes_aliases_encoding_and_extra_key_order() {
    let first = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=reality&type=xhttp&host=cdn.example.com&path=%2Fxp-xhttp&mode=stream-one&extra=%7B%22mode%22%3A%22auto%22%2C%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%2C%22cMaxReuseTimes%22%3A%220%22%7D%7D&sni=edge.example.com&alpn=h2&fp=chrome&pbk=public-key&sid=01234567";
    let equivalent = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?sid=01234567&alpn=h2&pbk=public-key&fp=chrome&sni=edge.example.com&extra=%7B%22xmux%22%3A%7B%22cMaxReuseTimes%22%3A%220%22%2C%22maxConnections%22%3A%221%22%7D%2C%22mode%22%3A%22auto%22%7D&mode=stream-one&path=/xp-xhttp&host=cdn.example.com&net=xhttp&security=reality";

    assert_eq!(
        normalize_single_proxy_key(first),
        normalize_single_proxy_key(equivalent),
        "xhttp aliases, query order, encoding, and JSON key order should be equivalent"
    );
}

#[test]
fn vless_xhttp_identity_normalizes_missing_mode_to_xray_default() {
    let omitted = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&path=%2Fprobe";
    let explicit_auto = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&path=%2Fprobe&mode=auto";

    assert_eq!(
        normalize_single_proxy_key(omitted),
        normalize_single_proxy_key(explicit_auto),
        "omitted XHTTP mode should share identity with Xray's explicit auto mode"
    );
}

#[test]
fn normalize_proxy_url_entries_deduplicates_equivalent_xhttp_links() {
    let first = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&path=%2Fprobe&mode=auto&extra=%7B%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%7D%7D";
    let equivalent = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?extra=%7B%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%7D%7D&mode=auto&path=/probe&net=xhttp&security=none";

    let normalized = normalize_proxy_url_entries(vec![first.to_string(), equivalent.to_string()]);

    assert_eq!(normalized.len(), 1);
    assert_eq!(
        normalize_single_proxy_key(&normalized[0]),
        normalize_single_proxy_key(first)
    );
}

#[test]
fn safe_forward_proxy_endpoint_url_hides_share_link_credentials() {
    let xhttp = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&extra=%7B%22secret%22%3A%22hidden%22%7D";

    assert_eq!(
        safe_forward_proxy_endpoint_url(Some(xhttp)),
        Some("vless://edge.example.com:443".to_string())
    );
    assert_eq!(
        safe_forward_proxy_endpoint_url(Some("http://user:password@proxy.example.com:8080")),
        Some("http://proxy.example.com:8080".to_string())
    );
}

#[test]
fn vless_xhttp_identity_marks_ambiguous_duplicate_query_fields() {
    let raw = Url::parse(
        "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&type=tcp",
    )
    .expect("duplicate xhttp fixture should parse");
    let identity = canonical_vless_share_link_identity(&raw);

    assert!(identity.contains("__invalid_xhttp_query"));
}

#[test]
fn vless_xhttp_identity_changes_when_effective_settings_change() {
    let base = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=reality&type=xhttp&host=cdn.example.com&path=%2Fxp-xhttp&mode=stream-one&extra=%7B%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%7D%7D&sni=edge.example.com&alpn=h2&fp=chrome&pbk=public-key&sid=01234567";
    let host_changed = base.replace("cdn.example.com", "other.example.com");
    let path_changed = base.replace("%2Fxp-xhttp", "%2Fother");
    let mode_changed = base.replace("stream-one", "stream-up");
    let extra_changed = base.replace(
        "%22maxConnections%22%3A%221%22",
        "%22maxConnections%22%3A%222%22",
    );
    let alpn_changed = base.replace("alpn=h2", "alpn=h3");

    for changed in [
        host_changed,
        path_changed,
        mode_changed,
        extra_changed,
        alpn_changed,
    ] {
        assert_ne!(
            normalize_single_proxy_key(base),
            normalize_single_proxy_key(&changed),
            "effective XHTTP or Reality identity fields must affect the stable key"
        );
    }
}

#[test]
fn vless_xhttp_invalid_extra_identity_does_not_retain_raw_value() {
    let raw = Url::parse(
        "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=none&type=xhttp&extra=%7B%22secret%22%3A%22do-not-retain",
    )
    .expect("invalid extra fixture should still be a valid URL");
    let identity = canonical_vless_share_link_identity(&raw);

    assert!(identity.contains("__invalid_xhttp_extra"));
    assert!(!identity.contains("do-not-retain"));
    assert!(!identity.contains("secret"));
}

#[test]
fn vless_xhttp_identity_keeps_legacy_saved_node_alias() {
    let raw = "vless://11111111-1111-1111-1111-111111111111@edge.example.com:443?security=reality&type=xhttp&host=cdn.example.com&path=%2Fxp-xhttp&mode=stream-one&extra=%7B%22xmux%22%3A%7B%22maxConnections%22%3A%221%22%7D%7D&sni=edge.example.com&fp=chrome&pbk=public-key&sid=01234567";
    let parsed = Url::parse(raw).expect("xhttp fixture should parse");
    let legacy_key = stable_forward_proxy_key(&canonical_share_link_identity(&parsed));
    let current_key = normalize_single_proxy_key(raw).expect("xhttp fixture should normalize");
    let aliases = legacy_bound_proxy_key_aliases(raw, ForwardProxyProtocol::Vless);

    assert_ne!(legacy_key, current_key);
    assert!(
        aliases.contains(&legacy_key),
        "saved XHTTP nodes should retain their historical key as a migration alias"
    );
}

#[test]
fn forward_proxy_validation_failure_codes_are_stable_and_safe() {
    let error = anyhow!(ForwardProxyValidationError::new(
        ForwardProxyValidationErrorCode::InvalidXhttpExtra,
        "xhttp extra contains invalid JSON",
    ));
    let (code, message) = classify_forward_proxy_validation_error(&error);
    assert_eq!(code, ForwardProxyValidationErrorCode::InvalidXhttpExtra);
    assert_eq!(message, "xhttp extra contains invalid JSON");

    let response = ForwardProxyCandidateValidationResponse::failed(code, message);
    let payload = serde_json::to_value(response).expect("validation response should serialize");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["code"], "invalid_xhttp_extra");
    assert!(
        payload["message"]
            .as_str()
            .unwrap_or_default()
            .contains("invalid JSON")
    );
    assert!(payload["normalizedValue"].is_null());
    assert!(payload["discoveredNodes"].is_null());
    assert!(payload["latencyMs"].is_null());

    let rejected = anyhow!("attempt 1 failed: validation probe returned status 502 Bad Gateway");
    assert_eq!(
        classify_forward_proxy_validation_error(&rejected),
        (
            ForwardProxyValidationErrorCode::UpstreamStatusRejected,
            "validation probe returned status 502".to_string()
        )
    );
}

#[test]
fn forward_proxy_validation_phase_codes_are_preserved_without_sensitive_details() {
    let cases = [
        (
            ForwardProxyValidationErrorCode::UnsupportedTransport,
            "unsupported proxy transport",
        ),
        (
            ForwardProxyValidationErrorCode::InvalidXhttpExtra,
            "xhttp extra contains invalid JSON",
        ),
        (
            ForwardProxyValidationErrorCode::UnsupportedXhttpMode,
            "xhttp mode is unsupported",
        ),
        (
            ForwardProxyValidationErrorCode::XrayConfigError,
            "xray rejected proxy configuration",
        ),
        (
            ForwardProxyValidationErrorCode::XrayStartupError,
            "xray proxy route failed to start",
        ),
        (
            ForwardProxyValidationErrorCode::ProxyConnectError,
            "proxy connection failed during validation",
        ),
        (
            ForwardProxyValidationErrorCode::ProxyTimeout,
            "proxy validation timed out",
        ),
        (
            ForwardProxyValidationErrorCode::UpstreamStatusRejected,
            "validation probe returned status 502",
        ),
        (
            ForwardProxyValidationErrorCode::ValidationFailed,
            "proxy validation failed",
        ),
    ];

    for (code, message) in cases {
        let error = anyhow!(ForwardProxyValidationError::new(code, message));
        assert_eq!(
            classify_forward_proxy_validation_error(&error),
            (code, message.to_string())
        );
        let payload = serde_json::to_value(ForwardProxyCandidateValidationResponse::failed(
            code, message,
        ))
        .expect("validation failure response should serialize");
        assert_eq!(payload["code"], code.as_str());
        assert_eq!(payload["ok"], false);
        assert!(payload["message"].as_str().is_some_and(|value| {
            !value.contains("11111111-1111-1111-1111-111111111111")
                && !value.contains("vless://")
                && !value.contains("extra-secret")
        }));
    }
}
