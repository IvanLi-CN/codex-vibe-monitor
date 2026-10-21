use std::{future::Future, pin::Pin};

use super::*;

type ViaPoolResponseFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Response, ProxyErrorResponse>> + Send + 'a>>;

mod auth;
mod forward_retry;
mod pool_forwarding;
mod pool_resolution;
mod route_semantics;

use auth::plain_proxy_error;
use pool_resolution::allocate_via_pool_invoke_id;

pub(crate) use auth::{
    extract_bearer_token, pool_route_response_status_is_success, request_matches_pool_route,
};
pub(crate) use forward_retry::send_forward_proxy_request_with_429_retry;
pub(crate) use pool_forwarding::proxy_openai_v1_via_pool;
pub(crate) use pool_resolution::*;
pub(crate) use route_semantics::*;
