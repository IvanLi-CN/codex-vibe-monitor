//! Upstream accounts domain entrypoint split into focused slices.
//!
//! Keep include order stable to preserve item resolution and behavior parity.

include!("core.rs");
include!("crud_group_notes.rs");
include!("imports_jobs_sse.rs");
include!("oauth_sessions_callbacks.rs");
include!("maintenance_dispatch.rs");
include!("sync.rs");
include!("routing.rs");
include!("tests.rs");
