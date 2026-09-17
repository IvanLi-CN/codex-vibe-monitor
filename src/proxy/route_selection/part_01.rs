macro_rules! pool_account_wait_request {
    (
        $state:expr,
        $sticky_key:expr,
        $requested_model:expr,
        $binding_constraint:expr,
        $conversation_override:expr,
        $wait_deadline:expr,
        $total_timeout_deadline:expr,
        $endpoint:expr,
        $image_intent:expr,
        $codex_imagegen_request:expr,
        $reservation_key:expr $(,)?,
    ) => {
        PoolAccountWaitRequest {
            state: $state,
            sticky_key: $sticky_key,
            requested_model: $requested_model,
            excluded_ids: &[],
            excluded_upstream_route_keys: &HashSet::new(),
            required_upstream_route_key: None,
            binding_constraint: $binding_constraint,
            conversation_override: $conversation_override,
            wait_for_no_available: true,
            wait_deadline: $wait_deadline,
            total_timeout_deadline: $total_timeout_deadline,
            endpoint: $endpoint,
            image_intent: $image_intent,
            codex_imagegen_request: $codex_imagegen_request,
            reservation_key: $reservation_key,
        }
    };
}
