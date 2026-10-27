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

pub(crate) fn optional_postgres_test_database_url(test_name: &str) -> Option<String> {
    match std::env::var("POSTGRES_TEST_DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => Some(value),
        Ok(_) | Err(std::env::VarError::NotPresent) => {
            let message = format!(
                "PostgreSQL acceptance test {test_name} requires POSTGRES_TEST_DATABASE_URL"
            );
            if std::env::var("POSTGRES_ACCEPTANCE_REQUIRED").as_deref() == Ok("1") {
                panic!("{message}; status=blocked result=not-run");
            }
            eprintln!("skipping optional PostgreSQL test: {message}");
            None
        }
        Err(error) => {
            let message = format!(
                "PostgreSQL acceptance test {test_name} cannot read POSTGRES_TEST_DATABASE_URL: {error}"
            );
            if std::env::var("POSTGRES_ACCEPTANCE_REQUIRED").as_deref() == Ok("1") {
                panic!("{message}; status=blocked result=not-run");
            }
            eprintln!("skipping optional PostgreSQL test: {message}");
            None
        }
    }
}
