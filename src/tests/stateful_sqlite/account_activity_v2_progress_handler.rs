use super::*;

#[tokio::test]
async fn account_activity_v2_priority_selection_skips_interrupted_sqlite_round() {
    let state = test_state_with_openai_base(
        Url::parse("https://api.openai.com/").expect("valid upstream base url"),
    )
    .await;
    let current_hour_epoch = align_bucket_epoch(Utc::now().timestamp(), 3_600, 0);
    sqlx::query(
        r#"
        INSERT INTO codex_invocations (
            invoke_id, occurred_at, source, status, total_tokens, payload, raw_response
        )
        VALUES ('v2-interrupted-selection', ?1, ?2, 'success', 1, ?3, '{}')
        "#,
    )
    .bind(format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local(),
    ))
    .bind(SOURCE_PROXY)
    .bind(r#"{"upstreamAccountId":42}"#)
    .execute(&state.pool)
    .await
    .expect("insert selection interruption fixture");

    let progress_probe = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started_at = std::time::Instant::now();
    let outcome = crate::select_active_account_activity_v2_priority_buckets_with_deadline(
        &state.pool,
        current_hour_epoch,
        started_at,
        started_at + std::time::Duration::from_secs(1),
        crate::ActiveAccountActivityV2ProgressHandlerOptions::new(
            Some(progress_probe.clone()),
            1,
            true,
        ),
    )
    .await
    .expect("handle interrupted selection round");
    assert!(outcome.is_none());
    assert!(progress_probe.load(std::sync::atomic::Ordering::Relaxed));
}

#[tokio::test]
async fn account_activity_v2_priority_selection_cancellation_does_not_poison_sqlite_pool() {
    let temp_dir = make_temp_test_dir("account-activity-v2-progress-handler-cancel");
    let db_path = temp_dir.join("state.sqlite");
    let connect_options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .expect("connect single-connection file-backed sqlite pool");
    crate::ensure_schema(&pool)
        .await
        .expect("initialize cancellation regression schema");
    sqlx::query("CREATE TABLE progress_handler_reuse_probe (value INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .expect("create persistent reuse probe");
    sqlx::query(
        "WITH RECURSIVE values_to_insert(value) AS (\
             VALUES (1) UNION ALL SELECT value + 1 FROM values_to_insert WHERE value < 2000\
         ) INSERT INTO progress_handler_reuse_probe SELECT value FROM values_to_insert",
    )
    .execute(&pool)
    .await
    .expect("seed persistent reuse probe");

    let (installed_tx, installed_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let started_at = std::time::Instant::now();
    let selection_deadline = started_at + std::time::Duration::from_millis(100);
    let selection_pool = pool.clone();
    let selection = tokio::spawn(async move {
        crate::select_active_account_activity_v2_priority_buckets_with_deadline(
            &selection_pool,
            align_bucket_epoch(Utc::now().timestamp(), 3_600, 0),
            started_at,
            selection_deadline,
            crate::ActiveAccountActivityV2ProgressHandlerOptions::new(None, 1, false)
                .pause_after_handler_install(
                    crate::ActiveAccountActivityV2ProgressHandlerTestPause {
                        installed: installed_tx,
                        resume: resume_rx,
                    },
                ),
        )
        .await
    });
    installed_rx
        .await
        .expect("signal after installing sqlite progress handler");
    selection.abort();
    assert!(
        selection
            .await
            .expect_err("selection task is cancelled")
            .is_cancelled()
    );
    drop(resume_tx);
    tokio::time::sleep_until(tokio::time::Instant::from_std(
        selection_deadline + std::time::Duration::from_millis(25),
    ))
    .await;

    let total: i64 = sqlx::query_scalar("SELECT SUM(value) FROM progress_handler_reuse_probe")
        .fetch_one(&pool)
        .await
        .expect("query after cancelled progress-handler selection");
    assert_eq!(total, 2_001_000);

    sqlx::query("CREATE TEMP TABLE normal_reuse_probe (value INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .expect("create normal-reuse probe on the pooled connection");
    sqlx::query("INSERT INTO normal_reuse_probe VALUES (17)")
        .execute(&pool)
        .await
        .expect("seed normal-reuse probe");
    let reuse_started_at = std::time::Instant::now();
    let selected = crate::select_active_account_activity_v2_priority_buckets_with_deadline(
        &pool,
        align_bucket_epoch(Utc::now().timestamp(), 3_600, 0),
        reuse_started_at,
        reuse_started_at + std::time::Duration::from_secs(2),
        crate::ActiveAccountActivityV2ProgressHandlerOptions::new(None, 1_000, false),
    )
    .await
    .expect("complete selection and remove sqlite progress handler")
    .expect("selection remains inside its budget");
    assert!(selected.is_empty());
    let sentinel: i64 = sqlx::query_scalar("SELECT value FROM normal_reuse_probe")
        .fetch_one(&pool)
        .await
        .expect("reuse the same connection after normal cleanup");
    assert_eq!(sentinel, 17);

    pool.close().await;
    std::fs::remove_dir_all(&temp_dir).expect("remove cancellation regression database");
}

#[tokio::test]
async fn account_activity_v2_priority_selection_internal_timeout_closes_installed_handler() {
    let temp_dir = make_temp_test_dir("account-activity-v2-progress-handler-timeout");
    let db_path = temp_dir.join("state.sqlite");
    let connect_options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(connect_options)
        .await
        .expect("connect single-connection file-backed sqlite pool");
    crate::ensure_schema(&pool)
        .await
        .expect("initialize internal-timeout regression schema");

    let (installed_tx, installed_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    let started_at = std::time::Instant::now();
    let selection_deadline = started_at + std::time::Duration::from_millis(100);
    let selection_pool = pool.clone();
    let selection = tokio::spawn(async move {
        crate::select_active_account_activity_v2_priority_buckets_with_deadline(
            &selection_pool,
            align_bucket_epoch(Utc::now().timestamp(), 3_600, 0),
            started_at,
            selection_deadline,
            crate::ActiveAccountActivityV2ProgressHandlerOptions::new(None, 1, false)
                .pause_after_handler_install(
                    crate::ActiveAccountActivityV2ProgressHandlerTestPause {
                        installed: installed_tx,
                        resume: resume_rx,
                    },
                ),
        )
        .await
    });
    installed_rx
        .await
        .expect("signal after installing sqlite progress handler");
    tokio::time::sleep_until(tokio::time::Instant::from_std(selection_deadline)).await;
    resume_tx.send(()).expect("resume selection after deadline");
    assert_eq!(
        selection
            .await
            .expect("selection task completes")
            .expect("selection remains fail-soft"),
        None,
        "internal deadline keeps the existing deferred result"
    );

    let result: i64 = sqlx::query_scalar("SELECT 42")
        .fetch_one(&pool)
        .await
        .expect("query after internal timeout");
    assert_eq!(result, 42);

    pool.close().await;
    std::fs::remove_dir_all(&temp_dir).expect("remove internal-timeout regression database");
}
