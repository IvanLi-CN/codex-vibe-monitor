#[cfg(test)]
mod tests {
    use super::super::*;
    #[allow(unused_imports)]
    use crate::*;

    async fn resolve_pool_account_for_request(
        state: &AppState,
        sticky_key: Option<&str>,
        excluded_ids: &[i64],
        excluded_upstream_route_keys: &std::collections::HashSet<String>,
    ) -> Result<PoolAccountResolution> {
        super::super::resolve_pool_account_for_request(
            state,
            sticky_key,
            None,
            excluded_ids,
            excluded_upstream_route_keys,
        )
        .await
    }

    async fn resolve_pool_account_for_request_with_binding_constraint(
        state: &AppState,
        sticky_key: Option<&str>,
        excluded_ids: &[i64],
        excluded_upstream_route_keys: &std::collections::HashSet<String>,
        binding_constraint: Option<&PromptCacheConversationBindingConstraint>,
    ) -> Result<PoolAccountResolution> {
        super::super::resolve_pool_account_for_request_with_binding_constraint(
            state,
            sticky_key,
            None,
            excluded_ids,
            excluded_upstream_route_keys,
            binding_constraint,
        )
        .await
    }

    include!("tests_part_1.rs");
    include!("tests_part_2.rs");
    include!("tests_part_3.rs");
    include!("tests_part_4.rs");
    include!("tests_part_5.rs");
    include!("tests_part_6.rs");
    include!("tests_part_7.rs");
}
