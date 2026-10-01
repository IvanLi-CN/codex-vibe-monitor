use super::*;

#[tokio::test]
async fn background_startup_hourly_rollup_bootstrap_finishes_orphaned_test_history() {
    let state = test_state_from_config(test_config(), false).await;
    let started_at_from = format_utc_iso_millis(Utc::now());
    let task_run = begin_system_task_run(
        &state.pool,
        SystemTaskKind::HourlyRollupBootstrap,
        "startup",
        Some("background hourly rollup bootstrap started".to_string()),
    )
    .await
    .expect("record startup bootstrap task history");
    let matching_id = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM system_task_runs
        WHERE task_kind = ?1
          AND trigger_kind = 'startup'
          AND status = ?2
          AND summary = 'background hourly rollup bootstrap started'
          AND started_at >= ?3
        ORDER BY id DESC
        LIMIT 1
        "#,
    )
    .bind("hourly_rollup_bootstrap")
    .bind(SystemTaskStatus::Running.as_str())
    .bind(&started_at_from)
    .fetch_optional(&state.pool)
    .await
    .expect("find just-recorded startup bootstrap history");
    assert_eq!(matching_id, Some(task_run.id));
    state.shutdown.cancel();

    crate::runtime::finish_orphaned_startup_hourly_rollup_bootstrap_task(
        state.as_ref(),
        &state.shutdown,
        &started_at_from,
    )
    .await;
    assert_eq!(
        state
            .sqlite_batch_writer
            .accounting_snapshot()
            .pending_depth,
        1,
        "orphan recovery should enqueue its terminal task-history write"
    );
    state
        .sqlite_batch_writer
        .flush_now(&state.pool)
        .await
        .expect("flush deferred cancellation task-history finish");

    let (status, summary, detail) = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        r#"
        SELECT status, summary, detail
        FROM system_task_runs
        WHERE id = ?1
        "#,
    )
    .bind(task_run.id)
    .fetch_one(&state.pool)
    .await
    .expect("read terminal startup bootstrap history");
    assert_eq!(status, "skipped");
    assert!(
        summary
            .as_deref()
            .is_some_and(|summary| summary.contains("cancelled before"))
    );
    assert!(detail.is_none());
}
