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
