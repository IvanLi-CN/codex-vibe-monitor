use super::*;
use crate::ObservabilityRuntime;

#[tokio::test]
async fn maintenance_store_writes_record_results_duration_and_cancellation() {
    let runtime = ObservabilityRuntime::new(true);

    crate::observability::contention::with_performance(runtime.clone(), async {
        observe_store_write(StoreOperation::Progress, async {
            Ok::<(), anyhow::Error>(())
        })
        .await
        .expect("successful store write");
        observe_store_write(StoreOperation::Progress, async {
            Err::<(), _>(anyhow::anyhow!("fixture store failure"))
        })
        .await
        .expect_err("fixture store failure");
        observe_store_write(StoreOperation::Progress, async {
            Err::<(), _>(anyhow::anyhow!("database is busy"))
        })
        .await
        .expect_err("busy store write");
        observe_store_write(StoreOperation::Progress, async {
            Err::<(), _>(anyhow::anyhow!("database is locked"))
        })
        .await
        .expect_err("locked store write");
    })
    .await;

    let mut pending = Box::pin(crate::observability::contention::with_performance(
        runtime.clone(),
        observe_store_write(
            StoreOperation::Cleanup,
            std::future::pending::<anyhow::Result<()>>(),
        ),
    ));
    assert!(futures_util::FutureExt::now_or_never(pending.as_mut()).is_none());
    drop(pending);

    let text = runtime.render();
    for outcome in ["committed", "error", "busy", "locked"] {
        assert!(
            text.contains(&format!(
                "cvm_maintenance_store_writes_total{{operation=\"progress\",outcome=\"{outcome}\"}} 1"
            )),
            "missing store outcome {outcome}: {text}"
        );
    }
    assert!(text.contains(
        "cvm_maintenance_store_writes_total{operation=\"cleanup\",outcome=\"cancelled\"} 1"
    ));
    assert!(
        text.contains(
            "cvm_maintenance_store_write_duration_seconds_count{operation=\"progress\"} 4"
        )
    );
    assert!(
        text.contains(
            "cvm_maintenance_store_write_duration_seconds_count{operation=\"cleanup\"} 1"
        )
    );
}

#[tokio::test]
async fn managed_dispatcher_noop_writes_record_noop_outcomes() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:?cache=shared")
        .await
        .expect("connect maintenance store test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");
    let runtime = ObservabilityRuntime::new(true);

    crate::observability::contention::with_performance(runtime.clone(), async {
        assert_eq!(
            store
                .enqueue_due_runs()
                .await
                .expect("probe scheduled runs"),
            0
        );
        assert!(
            store
                .claim_requested_run()
                .await
                .expect("claim requested run")
                .is_none()
        );
    })
    .await;

    let text = runtime.render();
    assert!(
        text.contains("cvm_maintenance_store_writes_total{operation=\"wake\",outcome=\"noop\"} 1")
    );
    assert!(
        text.contains("cvm_maintenance_store_writes_total{operation=\"run\",outcome=\"noop\"} 1")
    );
    assert!(
        !text.contains(
            "cvm_maintenance_store_writes_total{operation=\"wake\",outcome=\"committed\"}"
        )
    );
    assert!(
        !text.contains(
            "cvm_maintenance_store_writes_total{operation=\"run\",outcome=\"committed\"}"
        )
    );

    store
        .set_enabled("startup_backfill.proxy_usage", true)
        .await
        .expect("enable scheduled task for retirement fixture");
    store
        .request_run("startup_backfill.proxy_usage")
        .await
        .expect("queue scheduled task for retirement fixture");
    store
        .set_enabled("startup_backfill.proxy_usage", false)
        .await
        .expect("disable scheduled task for retirement fixture");

    crate::observability::contention::with_performance(runtime.clone(), async {
        assert!(
            store
                .claim_requested_run()
                .await
                .expect("retire requested run")
                .is_none()
        );
    })
    .await;

    let text = runtime.render();
    assert!(
        text.contains("cvm_maintenance_store_writes_total{operation=\"run\",outcome=\"noop\"} 2")
    );
    assert!(text.contains(
        "cvm_maintenance_store_writes_total{operation=\"cleanup\",outcome=\"committed\"} 1"
    ));
    assert!(
        !text
            .contains("cvm_maintenance_store_writes_total{operation=\"cleanup\",outcome=\"noop\"}")
    );
    let retired_status: String = sqlx::query_scalar(
        "SELECT status FROM managed_task_runs WHERE task_key='startup_backfill.proxy_usage'",
    )
    .fetch_one(&store.pool)
    .await
    .expect("load retired requested run status");
    assert_eq!(retired_status, "failed");
}

#[tokio::test]
async fn cached_prompt_cache_control_initialization_does_not_record_store_write() {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("connect maintenance store test pool");
    let store = MaintenanceStore::from_pool(pool);
    store
        .initialize_schema_for_test()
        .await
        .expect("initialize maintenance store schema");
    let task_key = "startup_backfill.prompt_cache_conversations_materialization";
    let task_name = "prompt_cache_conversations_materialization";
    let runtime = ObservabilityRuntime::new(true);

    crate::observability::contention::with_performance(runtime.clone(), async {
        store
            .initialize_prompt_cache_materialization_control(task_key, task_name)
            .await
            .expect("initialize prompt-cache control");
        store
            .initialize_prompt_cache_materialization_control(task_key, task_name)
            .await
            .expect("read cached prompt-cache control");
    })
    .await;

    let text = runtime.render();
    assert!(text.contains(
        "cvm_maintenance_store_writes_total{operation=\"control\",outcome=\"committed\"} 1"
    ));
    assert!(
        text.contains(
            "cvm_maintenance_store_write_duration_seconds_count{operation=\"control\"} 1"
        )
    );
}
