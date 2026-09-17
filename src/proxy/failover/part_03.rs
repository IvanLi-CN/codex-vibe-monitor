fn boxed_send_pool_request_with_failover_and_binding_constraint_inner<'a>(
    request: PoolFailoverBindingRequest<'a>,
) -> Pin<Box<dyn Future<Output = Result<PoolUpstreamResponse, PoolUpstreamError>> + Send + 'a>> {
    Box::pin(send_pool_request_with_failover_and_binding_constraint_inner(request))
}

fn pool_upstream_413_retry_allowed(
    status: StatusCode,
    priority_handoff_admitted: bool,
    retried_upstream_413_for_account: bool,
) -> bool {
    status == StatusCode::PAYLOAD_TOO_LARGE
        && !priority_handoff_admitted
        && !retried_upstream_413_for_account
}

#[cfg(test)]
mod request_compression_tests {
    use super::*;

    #[test]
    fn forwarded_oauth_request_compression_is_recorded_as_passthrough() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));

        assert_eq!(
            forwarded_request_compression_observation(&headers),
            Some(("zstd", "passthrough"))
        );
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("x-gzip"));
        assert_eq!(
            forwarded_request_compression_observation(&headers),
            Some(("gzip", "passthrough"))
        );
        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                true,
                RequestCompressionAlgorithm::Identity,
            ),
            Some("gzip")
        );
    }

    #[test]
    fn terminal_pool_request_compression_uses_actual_forwarding_mode() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));

        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                false,
                RequestCompressionAlgorithm::Follow,
            ),
            Some("gzip")
        );
        assert_eq!(
            resolved_pool_request_compression_algorithm(
                &headers,
                false,
                RequestCompressionAlgorithm::Zstd,
            ),
            Some("zstd")
        );
    }

    #[test]
    fn terminal_pool_request_compression_keeps_unknown_attempt_metadata_unknown() {
        assert_eq!(
            resolve_terminal_request_compression_algorithm(Some(None), Some("zstd".to_string())),
            None
        );
        assert_eq!(
            resolve_terminal_request_compression_algorithm(
                Some(Some("gzip".to_string())),
                Some("zstd".to_string())
            ),
            Some("gzip".to_string())
        );
        assert_eq!(
            resolve_terminal_request_compression_algorithm(None, Some("zstd".to_string())),
            Some("zstd".to_string())
        );
    }

    #[test]
    fn priority_handoff_disables_upstream_413_retry_budget() {
        assert!(!pool_upstream_413_retry_allowed(
            StatusCode::PAYLOAD_TOO_LARGE,
            true,
            false,
        ));
        assert!(pool_upstream_413_retry_allowed(
            StatusCode::PAYLOAD_TOO_LARGE,
            false,
            false,
        ));
        assert!(!pool_upstream_413_retry_allowed(
            StatusCode::PAYLOAD_TOO_LARGE,
            false,
            true,
        ));
    }
}
