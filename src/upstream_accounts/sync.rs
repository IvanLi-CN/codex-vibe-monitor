use super::*;

#[path = "sync_account_imports_tags.rs"]
mod sync_account_imports_tags;
#[path = "sync_group_sessions.rs"]
mod sync_group_sessions;
#[path = "sync_mailbox_and_filters.rs"]
mod sync_mailbox_and_filters;
#[path = "sync_oauth_crypto_utils.rs"]
mod sync_oauth_crypto_utils;
#[path = "sync_routing_status.rs"]
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
