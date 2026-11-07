pub(crate) use aes_gcm::Aes256Gcm;
pub(crate) use aes_gcm::aead::{Aead, KeyInit};
pub(crate) use axum::body::to_bytes;
pub(crate) use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
pub(crate) use brotli::CompressorWriter;
pub(crate) use chrono::Timelike;
pub(crate) use flate2::write::{DeflateEncoder, ZlibEncoder};
pub(crate) use rand::RngCore;
pub(crate) use rand::rngs::OsRng;
pub(crate) use sqlx::SqlitePool;
pub(crate) use sqlx::error::DatabaseError;
pub(crate) use sqlx::error::ErrorKind;
pub(crate) use std::sync::Mutex as StdMutex;
pub(crate) use tokio::sync::Notify;

mod timing;

pub(crate) use timing::{test_timing_span, test_timing_target, timed_test_phase};

// Synthetic credentials for tests which intentionally exercise private production routes.
const TEST_ADMIN_USERNAME: &str = "synthetic-admin";
const TEST_ADMIN_PASSWORD: &str = "synthetic-admin-fixture-only";

pub(crate) fn enable_test_admin_auth(state: &mut std::sync::Arc<crate::AppState>) {
    let state = std::sync::Arc::get_mut(state)
        .expect("configure test authentication before sharing the application state");
    state.config.admin_auth = Some(
        crate::config::AdminAuthConfig::from_credentials(
            TEST_ADMIN_USERNAME.to_string(),
            TEST_ADMIN_PASSWORD,
        )
        .expect("valid synthetic admin credentials"),
    );
}

pub(crate) fn test_admin_authorization() -> axum::http::HeaderValue {
    let encoded = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        format!("{TEST_ADMIN_USERNAME}:{TEST_ADMIN_PASSWORD}"),
    );
    axum::http::HeaderValue::from_str(&format!("Basic {encoded}"))
        .expect("valid synthetic admin authorization")
}

pub(crate) fn test_admin_request() -> axum::http::request::Builder {
    axum::http::Request::builder().header(
        axum::http::header::AUTHORIZATION,
        test_admin_authorization(),
    )
}
