use super::*;

#[tokio::test]
async fn timeseries_startup_recovery_empty_projection_invalidation_completes_without_rows() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    sqlx::query("DELETE FROM timeseries_minute_projection_v2")
        .execute(&state.pool)
        .await
        .expect("clear minute projection candidates");
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET generation = generation + 1, invalidation_pending = 1 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&state.pool)
    .await
    .expect("publish empty invalidation marker");

    let outcome = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
        &CancellationToken::new(),
    )
    .await
    .expect("empty coverage invalidation should complete");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );

    let pending = sqlx::query_scalar::<_, i64>(
        "SELECT invalidation_pending FROM timeseries_minute_projection_v2_recovery WHERE consumer = 'timeseries_minute_v2'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("load empty invalidation marker");
    assert_eq!(pending, 0);
    let projection_count =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM timeseries_minute_projection_v2")
            .fetch_one(&state.pool)
            .await
            .expect("count empty minute projections");
    assert_eq!(projection_count, 0);
}

#[tokio::test]
async fn timeseries_startup_recovery_invalidates_multiple_projection_batches() {
    let _projection_write_guard = TIMESERIES_MINUTE_PROJECTION_WRITE_TEST_LOCK.lock().await;
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    for minute_start_epoch in 0_i64..257 {
        sqlx::query(
            "INSERT INTO timeseries_minute_projection_v2 (minute_start_epoch, source_scope, upstream_account_key, aggregate_json, total_latency_samples_json, first_byte_samples_json, first_response_byte_total_samples_json, first_token_samples_json, max_row_id, coverage_state) VALUES (?1, 'all', -1, '{}', '[]', '[]', '[]', '[]', 0, 'ready')",
        )
        .bind(minute_start_epoch)
        .execute(&state.pool)
        .await
        .expect("seed multiple invalidation batches");
    }
    sqlx::query(
        "UPDATE timeseries_minute_projection_v2_recovery SET generation = generation + 1, invalidation_pending = 1 WHERE consumer = 'timeseries_minute_v2'",
    )
    .execute(&state.pool)
    .await
    .expect("publish multi-batch invalidation marker");

    let outcome = crate::api::prepare_timeseries_minute_projection_after_restart(
        state.as_ref(),
        &CancellationToken::new(),
    )
    .await
    .expect("multi-batch coverage invalidation should complete");
    assert_eq!(
        outcome,
        crate::api::TimeseriesMinuteProjectionFlushOutcome::Flushed
    );
    let ready_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE coverage_state = 'ready'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count remaining ready projection rows");
    assert_eq!(ready_count, 0);
    let warming_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM timeseries_minute_projection_v2 WHERE coverage_state = 'warming'",
    )
    .fetch_one(&state.pool)
    .await
    .expect("count invalidated projection rows");
    assert_eq!(warming_count, 257);
}
