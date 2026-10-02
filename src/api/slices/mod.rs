use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "Existing internal response adapters preserve established call-site and payload contracts."
)]
mod error_distribution_and_sse;
#[expect(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "Existing internal query adapters preserve established call-site contracts."
)]
mod invocations_and_summary;
mod long_term_stats_api;
mod model_management;
mod prompt_cache_and_timeseries;
mod public_blog_runtime;
mod settings_models_and_cache;
mod subscriptions;
mod system_routes_and_tasks;

pub(crate) use error_distribution_and_sse::*;
pub(crate) use invocations_and_summary::*;
pub(crate) use long_term_stats_api::*;
pub(crate) use model_management::*;
pub(crate) use prompt_cache_and_timeseries::prompt_cache_and_timeseries_shared;
pub(crate) use prompt_cache_and_timeseries::*;
pub(crate) use public_blog_runtime::*;
pub(crate) use settings_models_and_cache::*;
pub(crate) use subscriptions::*;
pub(crate) use system_routes_and_tasks::*;

pub(crate) fn build_settings_routes(router: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    router
        .route("/api/settings", get(get_settings))
        .route(
            "/api/settings/external-api-keys",
            get(list_external_api_keys).post(create_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/{id}/rotate",
            post(rotate_external_api_key),
        )
        .route(
            "/api/settings/external-api-keys/{id}/disable",
            post(disable_external_api_key),
        )
        .route(
            "/api/settings/proxy-models",
            any(removed_proxy_model_settings_endpoint),
        )
        .route("/api/settings/proxy", put(put_proxy_settings))
        .route(
            "/api/settings/models/sync/preview",
            post(post_models_sync_preview),
        )
        .route(
            "/api/settings/models/sync/state",
            get(get_models_sync_state).patch(patch_models_sync_state),
        )
        .route(
            "/api/settings/models/sync/apply",
            post(post_models_sync_apply),
        )
        .route("/api/settings/models/preset", put(put_managed_model_preset))
        .route("/api/settings/models", delete(delete_managed_model))
        .route(
            "/api/settings/forward-proxy",
            put(put_forward_proxy_settings),
        )
        .route(
            "/api/settings/forward-proxy/validate",
            post(post_forward_proxy_candidate_validation),
        )
        .route(
            "/api/settings/forward-proxy/refresh-subscriptions",
            post(post_forward_proxy_refresh_subscriptions),
        )
        .route(
            "/api/settings/forward-proxy/nodes/{proxy_key}/test-stream",
            get(stream_forward_proxy_node_latency_test),
        )
        .route(
            "/api/settings/forward-proxy/nodes/test-stream",
            get(stream_forward_proxy_nodes_latency_test),
        )
        .route("/api/settings/pricing", put(put_pricing_settings))
}
