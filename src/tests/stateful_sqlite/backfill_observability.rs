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
