use super::*;

#[tokio::test]
async fn pool_attempt_backfill_skips_rows_with_existing_public_ids() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect sqlite test pool");
    crate::schema::ensure_schema(&pool)
        .await
        .expect("initialize schema");

    for (attempt_public_id, invoke_id) in
        [(Some("already-assigned"), "existing"), (None, "missing")]
    {
        sqlx::query(
            "INSERT INTO pool_upstream_request_attempts
             (attempt_public_id, invoke_id, occurred_at, endpoint, route_mode,
              attempt_index, distinct_account_index, same_account_retry_index, status)
             VALUES (?1, ?2, '2026-10-19T00:00:00.000Z', '/v1/responses', 'pool', 0, 0, 0, 'success')",
        )
        .bind(attempt_public_id)
        .bind(invoke_id)
        .execute(&pool)
        .await
        .expect("insert backfill fixture");
    }

    let outcome = crate::backfill_pool_upstream_request_attempt_public_ids_from_cursor(
        &pool,
        0,
        Some(10),
        Some(Duration::from_secs(1)),
    )
    .await
    .expect("backfill missing public id");

    assert_eq!(outcome.summary.scanned, 1);
    assert_eq!(outcome.summary.updated, 1);
    let existing_id: String = sqlx::query_scalar(
        "SELECT attempt_public_id FROM pool_upstream_request_attempts WHERE invoke_id='existing'",
    )
    .fetch_one(&pool)
    .await
    .expect("load existing public id");
    assert_eq!(existing_id, "already-assigned");
}
