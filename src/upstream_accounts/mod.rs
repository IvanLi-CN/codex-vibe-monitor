//! Upstream accounts domain entrypoint split into focused slices.

use super::*;

mod core;
mod crud_group_notes;
mod external_api_integration;
mod imports_jobs_sse;
mod maintenance_dispatch;
#[expect(
    clippy::too_many_arguments,
    reason = "OAuth callback persistence adapters mirror session metadata fields."
)]
mod oauth_sessions_callbacks;
mod routing;
mod sync;
#[cfg(test)]
mod tests;

pub(crate) use core::*;
pub(crate) use crud_group_notes::*;
pub(crate) use external_api_integration::*;
pub(crate) use imports_jobs_sse::*;
pub(crate) use maintenance_dispatch::*;
pub(crate) use oauth_sessions_callbacks::*;
pub(crate) use routing::*;
pub(crate) use sync::*;
