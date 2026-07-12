use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "Proxy dispatch adapters preserve established call-site contracts."
)]
mod dispatch;
#[expect(
    clippy::too_many_arguments,
    reason = "Failover adapters preserve established call-site contracts."
)]
mod failover;
mod payload_utils;
#[expect(
    clippy::too_many_arguments,
    reason = "Capture backfill adapters preserve established call-site contracts."
)]
mod raw_capture;
#[expect(
    clippy::too_many_arguments,
    clippy::large_enum_variant,
    reason = "Request-entry variants and adapters preserve established runtime contracts."
)]
mod request_entry;
#[expect(
    clippy::too_many_arguments,
    reason = "Route-selection adapters preserve established call-site contracts."
)]
mod route_selection;
mod stream_gate;
#[expect(
    clippy::too_many_arguments,
    reason = "Usage persistence adapters preserve established database call contracts."
)]
mod usage_persistence;
#[expect(
    clippy::too_many_arguments,
    reason = "WebSocket preparation adapters preserve established call-site contracts."
)]
mod websocket;

pub(crate) use dispatch::*;
pub(crate) use failover::*;
pub(crate) use payload_utils::*;
pub(crate) use raw_capture::*;
pub(crate) use request_entry::*;
pub(crate) use route_selection::*;
pub(crate) use stream_gate::*;
pub(crate) use usage_persistence::*;
pub(crate) use websocket::*;
