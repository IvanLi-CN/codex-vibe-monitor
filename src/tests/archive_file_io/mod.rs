#[allow(unused_imports)]
use super::*;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

type TestFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

type RetentionInvocationInserter = for<'a> fn(
    &'a SqlitePool,
    &'a str,
    &'a str,
    &'a str,
    &'a str,
    Option<&'a str>,
    &'a str,
    Option<&'a Path>,
    Option<&'a Path>,
    Option<i64>,
    Option<f64>,
) -> TestFuture<'a>;

#[allow(non_upper_case_globals)]
pub(crate) static insert_retention_invocation: RetentionInvocationInserter =
    |pool,
     invoke_id,
     occurred_at,
     source,
     status,
     payload,
     raw_response,
     request_raw_path,
     response_raw_path,
     total_tokens,
     cost| {
        Box::pin(async move {
            crate::tests::lightweight::insert_retention_invocation(
                pool,
                crate::tests::lightweight::RetentionInvocationFixture {
                    invoke_id,
                    occurred_at,
                    source,
                    status,
                    payload,
                    raw_response,
                    request_raw_path,
                    response_raw_path,
                    total_tokens,
                    cost,
                },
            )
            .await;
        })
    };

type RetentionPoolAttemptInserter = for<'a> fn(
    &'a SqlitePool,
    &'a str,
    &'a str,
    Option<i64>,
    i64,
    i64,
    i64,
    &'a str,
    Option<i64>,
    Option<&'a str>,
    Option<&'a str>,
    Option<&'a str>,
) -> TestFuture<'a>;

#[allow(non_upper_case_globals)]
pub(crate) static insert_retention_pool_upstream_request_attempt: RetentionPoolAttemptInserter =
    |pool,
     invoke_id,
     occurred_at,
     upstream_account_id,
     attempt_index,
     distinct_account_index,
     same_account_retry_index,
     status,
     http_status,
     failure_kind,
     started_at,
     finished_at| {
        Box::pin(async move {
            crate::tests::lightweight::insert_retention_pool_upstream_request_attempt(
                pool,
                crate::tests::lightweight::RetentionPoolAttemptFixture {
                    invoke_id,
                    occurred_at,
                    upstream_account_id,
                    attempt_index,
                    distinct_account_index,
                    same_account_retry_index,
                    status,
                    http_status,
                    failure_kind,
                    started_at,
                    finished_at,
                },
            )
            .await;
        })
    };

type HourlyRollupInserter =
    for<'a> fn(&'a SqlitePool, DateTime<Utc>, &'a str, i64, i64, i64, i64, f64) -> TestFuture<'a>;

#[allow(non_upper_case_globals)]
pub(crate) static insert_invocation_hourly_rollup_bucket: HourlyRollupInserter =
    |pool,
     bucket_start,
     source,
     total_count,
     success_count,
     failure_count,
     total_tokens,
     total_cost| {
        Box::pin(async move {
            crate::tests::stateful_sqlite::insert_invocation_hourly_rollup_bucket(
                pool,
                crate::tests::stateful_sqlite::HourlyRollupFixture {
                    bucket_start,
                    source,
                    total_count,
                    success_count,
                    failure_count,
                    total_tokens,
                    total_cost,
                    first_byte_samples: &[],
                    first_response_byte_total_samples: &[],
                },
            )
            .await;
        })
    };

mod archive_backfill_and_materialization;
#[expect(
    clippy::await_holding_lock,
    reason = "Mock reservation logs intentionally stay locked until async assertions observe requests."
)]
mod raw_payload_retention_and_compression;
