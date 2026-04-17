//! Upstream accounts domain entrypoint split into focused slices.

use crate::*;

include!("core.rs");
mod crud_group_notes;
mod external_api_integration;
mod imports_jobs_sse;
mod maintenance_dispatch;
mod oauth_sessions_callbacks;
mod routing;
include!("sync.rs");
#[cfg(test)]
mod tests;

pub(crate) use crud_group_notes::*;
pub(crate) use external_api_integration::*;
pub(crate) use imports_jobs_sse::*;
pub(crate) use maintenance_dispatch::*;
pub(crate) use oauth_sessions_callbacks::*;
pub(crate) use routing::*;
