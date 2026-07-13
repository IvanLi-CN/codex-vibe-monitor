use super::*;

#[path = "sync_account_imports_tags.rs"]
mod sync_account_imports_tags;
#[path = "sync_group_sessions.rs"]
#[expect(
    clippy::too_many_arguments,
    clippy::type_complexity,
    reason = "Group session adapters mirror persisted group metadata and usage row shapes."
)]
mod sync_group_sessions;
#[path = "sync_mailbox_and_filters.rs"]
#[expect(
    clippy::too_many_arguments,
    reason = "Routing-rule normalization mirrors persisted policy columns."
)]
mod sync_mailbox_and_filters;
#[path = "sync_oauth_crypto_utils.rs"]
mod sync_oauth_crypto_utils;
#[path = "sync_routing_status.rs"]
#[expect(
    clippy::too_many_arguments,
    reason = "Status transition adapters mirror persisted audit fields."
)]
mod sync_routing_status;

pub(crate) use sync_account_imports_tags::*;
pub(crate) use sync_group_sessions::*;
pub(crate) use sync_mailbox_and_filters::*;
pub(crate) use sync_oauth_crypto_utils::*;
pub(crate) use sync_routing_status::*;

pub(crate) fn normalize_upstream_image_tool_rewrite_mode(
    value: Option<&str>,
) -> Result<ImageToolRewriteMode, (StatusCode, String)> {
    sync_mailbox_and_filters::normalize_image_tool_rewrite_mode(value)
}
